//! A reusable single-line text input for gpui.
//!
//! Adapted from gpui-vue (`crates/gpui-vue-core/src/custom_elements/input.rs`,
//! Apache-2.0), which is itself derived from the native renderer in
//! [remorses/gpuix](https://github.com/remorses/gpuix) @
//! `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69` (Apache-2.0) and from
//! [zeronsh/comet](https://github.com/zeronsh/comet) (MIT) — the caret blink is
//! ported from Comet's `crates/ui/src/composer.rs`.
//! See gpui-vue's `THIRD_PARTY_NOTICES.md`.
//!
//! Where gpui-vue's (older) gpui API disagrees with the rev this workspace
//! builds against, the API usage follows gpui's own
//! `crates/gpui/examples/input.rs` (Apache-2.0, zed-industries/zed).
//!
//! # Shape of the API
//!
//! The framework keeps one [`Entity<TextInputState>`] per `<input>` node in a
//! side map and rebuilds the element tree every frame, so the split is:
//!
//! * [`TextInputState`] — everything that must survive a frame (content,
//!   selection, IME pre-edit, undo history, focus handle, caret blink, the echo
//!   ring).
//! * [`TextInputOptions`] — everything the CSS engine recomputes per frame
//!   (placeholder, colours, font size, disabled, tab index).
//! * [`TextInputHandlers`] — the per-frame event sinks (`input`,
//!   `beforeinput`, the composition events, `keydown`, `keyup`,
//!   `copy`/`cut`/`paste`); `keydown`, the clipboard hook and `beforeinput`
//!   run *before* the widget acts and can cancel it.
//! * [`text_input`] — builds the element for one frame out of the three.

mod boundaries;
pub mod config;
mod element;
mod state;

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, Hsla, InteractiveElement, IntoElement,
    KeyBinding, KeyDownEvent, KeyUpEvent, MouseButton, ParentElement, Pixels, SharedString,
    Styled, TextInputConfiguration, Window, actions, div, hsla, px,
};

pub use config::{InputAttributes, MappedConfiguration, configuration};
pub use state::TextInputState;

use element::TextElement;

actions!(
    vn_text_input,
    [
        Backspace,
        ClearSelection,
        Copy,
        Cut,
        Delete,
        DeleteToEnd,
        DeleteToStart,
        DeleteWordLeft,
        DeleteWordRight,
        End,
        Home,
        Left,
        Paste,
        Redo,
        Right,
        SelectAll,
        SelectEnd,
        SelectHome,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        ShowCharacterPalette,
        Undo,
        WordLeft,
        WordRight,
        // Multi-line only (`<textarea>`). `Newline` is bound to `enter` in the
        // TEXTAREA key context alone, so a single-line `<input>` still leaves
        // Return to the surrounding app (form submit, reka's typeahead).
        Down,
        Newline,
        SelectDown,
        SelectUp,
        Up,
    ]
);

/// The `key_context` the input's key bindings are scoped to. Exposed so an
/// embedder can write its own bindings against the same context.
pub const KEY_CONTEXT: &str = "VnTextInput";

/// The extra `key_context` a MULTI-LINE field carries. gpui parses a context
/// string as a SET of identifiers, so a textarea sets both and thereby
/// inherits every editing binding while adding the ones that only mean
/// something with more than one line: `enter`, `up`, `down`.
pub const TEXTAREA_KEY_CONTEXT: &str = "VnTextArea";
const BOTH_KEY_CONTEXTS: &str = "VnTextInput VnTextArea";

/// HTML's default `<textarea rows>`.
pub const DEFAULT_ROWS: usize = 2;

/// What a user edit does, in the Input Events vocabulary: the DOM's
/// `InputEvent.inputType`. The discriminant is the code the framework carries
/// across to JS, where `EDIT_TYPES` in `packages/vue-native/src/runtime.ts`
/// names it (in lockstep: `test/protocol-codes.test.ts` parses this enum).
///
/// Only what this widget can do is here. It has no drag-and-drop of text
/// (`insertFromDrop`), no spelling replacement (`insertReplacementText`) and no
/// formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum InputType {
    /// Typed text. Its data is the text.
    InsertText = 1,
    /// Return in a `<textarea>`.
    InsertLineBreak = 2,
    /// A paste. Its data is the clipboard's text, as a browser's plain-text
    /// field has it.
    InsertFromPaste = 3,
    /// The IME's pre-edit, and its commit. Its data is the composition's text.
    InsertCompositionText = 4,
    DeleteContentBackward = 5,
    DeleteContentForward = 6,
    DeleteWordBackward = 7,
    DeleteWordForward = 8,
    DeleteSoftLineBackward = 9,
    DeleteSoftLineForward = 10,
    DeleteByCut = 11,
    HistoryUndo = 12,
    HistoryRedo = 13,
}

impl InputType {
    /// The DOM's name for it.
    pub const fn name(self) -> &'static str {
        match self {
            InputType::InsertText => "insertText",
            InputType::InsertLineBreak => "insertLineBreak",
            InputType::InsertFromPaste => "insertFromPaste",
            InputType::InsertCompositionText => "insertCompositionText",
            InputType::DeleteContentBackward => "deleteContentBackward",
            InputType::DeleteContentForward => "deleteContentForward",
            InputType::DeleteWordBackward => "deleteWordBackward",
            InputType::DeleteWordForward => "deleteWordForward",
            InputType::DeleteSoftLineBackward => "deleteSoftLineBackward",
            InputType::DeleteSoftLineForward => "deleteSoftLineForward",
            InputType::DeleteByCut => "deleteByCut",
            InputType::HistoryUndo => "historyUndo",
            InputType::HistoryRedo => "historyRedo",
        }
    }

    /// The code the framework carries across to JS.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The IME's text: its `beforeinput` cannot be cancelled, and its events
    /// say `isComposing` (Input Events Level 1, which Chromium implements).
    pub const fn composing(self) -> bool {
        matches!(self, InputType::InsertCompositionText)
    }
}

/// One user edit, as `beforeinput` announces it and `input` reports it: its
/// type, and its `data` (the inserted text for the types that insert some,
/// `None` for the others).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputIntent {
    pub input_type: InputType,
    pub data: Option<String>,
}

impl InputIntent {
    /// An edit with no data: a deletion, a line break, undo or redo.
    pub fn bare(input_type: InputType) -> Self {
        Self { input_type, data: None }
    }

    pub fn with(input_type: InputType, data: &str) -> Self {
        Self { input_type, data: Some(data.to_owned()) }
    }
}

/// Called by [`TextInputState`] whenever the user's editing changed the
/// content; the framework forwards it to JS as an `input` event. The
/// [`InputIntent`] is the edit that did it.
///
/// Invoked deferred (via [`Window::defer`]), i.e. after the edit that caused it
/// has finished, so it is safe to call back into the same entity — including
/// [`TextInputState::sync_value`] — from inside it.
pub type OnChange = Rc<dyn Fn(&str, &InputIntent, &mut Window, &mut App)>;

/// Fired before every user edit, the native side of the DOM's `beforeinput`:
/// after the keystroke's `keydown` (and a paste's or cut's ClipboardEvent),
/// before the content changes. Returning `false` means the event was
/// cancelled: the widget makes no edit and reports no change. Not called when
/// the edit would do nothing (Backspace at the start of the value).
///
/// The IME's events ([`InputType::composing`]) cannot be cancelled; their
/// answer is ignored. The hook runs inside the widget's update, as
/// [`OnKeyDown`] does; what its handlers assign to the field is read back
/// ([`ControlledValue`]) before the edit, which then applies to it.
pub type OnBeforeInput = Rc<dyn Fn(&InputIntent, &mut Window, &mut App) -> bool>;

/// The three stages of an IME composition (UI Events §5.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositionPhase {
    Start,
    Update,
    End,
}

/// The DOM's `compositionstart` / `compositionupdate` / `compositionend`,
/// with their `data`: what the composition replaces at the start (usually
/// nothing), the pre-edit text at each update, the committed text at the end.
///
/// The order is Chromium's for one pre-edit: `compositionstart`, then
/// `beforeinput`, `compositionupdate`, the edit, `input`; the commit's
/// `compositionend` comes after its `input`. `Start` and `Update` run inside
/// the widget's update, as [`OnBeforeInput`] does; `End` is deferred, after the
/// `input` it follows. Every `Start` gets its `End`: a commit, a cancelled
/// pre-edit, the field losing focus, or a value assigned from outside in the
/// middle of a composition each end it.
pub type OnComposition = Rc<dyn Fn(CompositionPhase, &str, &mut Window, &mut App)>;

/// Fired for every keystroke that reaches this input *before* the widget acts
/// on it — the native equivalent of a DOM `keydown` listener.
///
/// Returning `false` means the JS side called `preventDefault()`: the widget
/// must not apply its default editing for that keystroke (no key binding
/// action, no character insertion). Returning `true` lets the widget's own
/// bindings (`ctrl-c`, `backspace`, …) and text input run as usual.
pub type OnKeyDown = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App) -> bool>;

/// Fired after a key is released. Fire-and-forget: a `keyup` has no default
/// action to cancel, so there is nothing to report back.
pub type OnKeyUp = Rc<dyn Fn(&KeyUpEvent, &mut Window, &mut App)>;

/// The three clipboard commands, whichever key gave them: `ctrl-c` /
/// `ctrl-insert`, `ctrl-x` / `shift-delete`, `ctrl-v` / `shift-insert`
/// (`cmd-…` on macOS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardCommand {
    Copy,
    Cut,
    Paste,
}

/// Fired when the user copies, cuts or pastes in this field, BEFORE the widget
/// touches the clipboard or the content: the native side of the DOM's `copy` /
/// `cut` / `paste` ClipboardEvents.
///
/// Returning `false` means the event was cancelled: the widget does nothing
/// at all (no copy, no deletion, no insertion). Writing a cancelled copy's
/// own data (`clipboardData.setData` + `preventDefault()`) to the system
/// clipboard is the hook's job, not the widget's. It is called outside any
/// update of the widget's state, so it may call back into it.
pub type OnClipboard = Rc<dyn Fn(ClipboardCommand, &mut Window, &mut App) -> bool>;

/// The framework's controlled value as it stands NOW: the string and its write
/// counter (see [`TextInputState::sync_value`]), or `None` when nothing
/// assigned one.
///
/// The widget reads it again after an [`OnKeyDown`] or [`OnClipboard`] hook
/// lets the key or command through, right before it acts. The hook's handlers
/// may have assigned the field (a chat composer clearing itself on Enter, a
/// handler rewriting the text), and a browser's default action edits what they
/// wrote, not what the field showed before the event. Without it the
/// assignment would only land at the next frame's sync, after the edit.
pub type ControlledValue = Rc<dyn Fn(&App) -> Option<(String, u32)>>;

/// The per-frame callback set the framework hands the widget. All optional:
/// the framework only fills in what the JS side actually listens for.
#[derive(Clone, Default)]
pub struct TextInputHandlers {
    /// User editing changed the content (`input`/`change`).
    pub on_change: Option<OnChange>,
    /// Before each user edit, and able to cancel it; see [`OnBeforeInput`].
    pub on_before_input: Option<OnBeforeInput>,
    /// The IME composition's three stages; see [`OnComposition`].
    pub on_composition: Option<OnComposition>,
    /// Pre-processing hook for keystrokes; see [`OnKeyDown`].
    pub on_key_down: Option<OnKeyDown>,
    pub on_key_up: Option<OnKeyUp>,
    /// Copy / cut / paste, before the widget acts; see [`OnClipboard`].
    pub on_clipboard: Option<OnClipboard>,
    /// Re-read after a hook, before the default action; see [`ControlledValue`].
    pub controlled_value: Option<ControlledValue>,
}

impl std::fmt::Debug for TextInputHandlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextInputHandlers")
            .field("on_change", &self.on_change.is_some())
            .field("on_before_input", &self.on_before_input.is_some())
            .field("on_composition", &self.on_composition.is_some())
            .field("on_key_down", &self.on_key_down.is_some())
            .field("on_key_up", &self.on_key_up.is_some())
            .field("on_clipboard", &self.on_clipboard.is_some())
            .field("controlled_value", &self.controlled_value.is_some())
            .finish()
    }
}

impl TextInputHandlers {
    /// The common case: only content changes are observed.
    pub fn with_change(on_change: OnChange) -> Self {
        Self { on_change: Some(on_change), ..Default::default() }
    }
}

/// Per-frame view parameters. Style comes from the framework's CSS engine;
/// nothing in here is remembered between frames.
#[derive(Clone, Debug)]
pub struct TextInputOptions {
    /// Shown, in `placeholder_color`, while the content is empty.
    pub placeholder: SharedString,
    pub text_color: Hsla,
    pub placeholder_color: Hsla,
    pub selection_color: Hsla,
    pub caret_color: Hsla,
    pub font_size: Pixels,
    /// Read-only: no typing, paste, cut, undo/redo or IME, and no caret.
    /// Focus, selection and copy still work.
    pub disabled: bool,
    /// DOM `tabindex`: `Some(n)` makes the input a tab stop at index `n`,
    /// `None` takes it out of the tab order.
    pub tab_index: Option<isize>,
    /// `<textarea>`: the content soft-wraps, Return inserts a newline instead
    /// of being dropped, `up`/`down` move by visual line, and the box scrolls
    /// vertically rather than horizontally.
    pub multiline: bool,
    /// The cascade gave this element no `color` of its own, so the editor
    /// should paint in the INHERITED one.
    ///
    /// `color` is an inherited property in CSS and vn-css folds no inherited
    /// properties, but gpui carries a text style down the PAINT walk — so the
    /// widget reads `window.text_style()` at prepaint instead of being handed a
    /// guess. Without it an `<input>` that styles only its border paints its
    /// text in the framework's fallback colour, which is white-on-white in a
    /// light theme.
    pub inherit_color: bool,
    /// CSS `text-align`. Applied as a content offset inside the box rather than
    /// at paint time, so the caret, the selection, click-to-position and the
    /// IME candidate window all move with the glyphs — see
    /// `TextInputState::align_offset`.
    pub text_align: gpui::TextAlign,
    /// `<textarea rows>` — the INTRINSIC height, in lines. As in HTML it is
    /// only a minimum: a CSS `height`/`min-height` still wins and content past
    /// it scrolls. Ignored unless `multiline`.
    pub rows: usize,
    /// What the platform should be told about this field's text assistance —
    /// the `autocorrect` / `autocapitalize` / `spellcheck` / `enterkeyhint`
    /// attributes, mapped by [`config::configuration`].
    ///
    /// gpui re-queries this every frame off the FOCUSED field's input handler
    /// and forwards it only on change, so it belongs with the per-frame options
    /// rather than with the persistent state: the attributes are reactive props
    /// and a `:spellcheck="…"` that flips has to take effect on the next frame.
    pub input_config: TextInputConfiguration,
    /// CSS `pointer-events: none` is in force here (the element's own, or an
    /// ancestor's — the property inherits).
    ///
    /// The field takes no press: neither its own mouse handler nor the
    /// automatic focus transfer gpui gives every element with a tracked focus
    /// handle. Dropping the tracked handle is what removes the second one, and
    /// it costs the field its TAB STOP — a divergence from a browser, where
    /// `pointer-events` is about the pointer only. It buys the idiom this
    /// exists for: a tab panel parked over the visible one as `absolute
    /// inset-x-0 top-0 opacity-0 pointer-events-none`, whose fields sit pixel
    /// for pixel on top of the live ones and would otherwise swallow every
    /// click and every keystroke aimed at them.
    pub inert: bool,
    /// `<input type="password">`: every character paints as a bullet, and the
    /// value cannot be copied or cut out of the field, as in a browser.
    pub masked: bool,
}

impl Default for TextInputOptions {
    fn default() -> Self {
        Self {
            placeholder: SharedString::default(),
            text_color: hsla(0.0, 0.0, 0.1, 1.0),
            placeholder_color: hsla(0.0, 0.0, 0.1, 0.4),
            selection_color: hsla(0.62, 1.0, 0.6, 0.35),
            caret_color: hsla(0.0, 0.0, 0.1, 1.0),
            font_size: px(14.0),
            disabled: false,
            tab_index: None,
            multiline: false,
            text_align: gpui::TextAlign::Left,
            rows: DEFAULT_ROWS,
            inherit_color: false,
            input_config: TextInputConfiguration::default(),
            inert: false,
            masked: false,
        }
    }
}

/// One-time app setup: the text input's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    cx.bind_keys(bindings());
}

/// Build the element for one frame.
///
/// `state` is the entity the framework keeps for this node, `options` the
/// styling for this frame, `handlers` the per-frame event sinks.
pub fn text_input(
    state: &Entity<TextInputState>,
    options: TextInputOptions,
    handlers: TextInputHandlers,
    cx: &mut App,
) -> AnyElement {
    let disabled = options.disabled;
    let multiline = options.multiline;
    let input_config = options.input_config.clone();
    let on_key_up = handlers.on_key_up.clone();
    let on_clipboard = handlers.on_clipboard.clone();
    // The focus handle carries the tab order (gpui reads `tab_index`/`tab_stop`
    // straight off the handle when the element is painted), so it has to be
    // updated before `track_focus` clones it below.
    let focus_handle = state.update(cx, |this, _| {
        this.set_tab_index(options.tab_index);
        this.set_frame_params(handlers, disabled, multiline, input_config);
        this.focus_handle()
    });

    let mut element = div().key_context(if multiline { BOTH_KEY_CONTEXTS } else { KEY_CONTEXT });
    if !options.inert {
        element = element.track_focus(&focus_handle);
    }
    let mut element = element
        .cursor(if disabled {
            CursorStyle::Arrow
        } else {
            CursorStyle::IBeam
        })
        .flex()
        .w_full()
        .min_w_0()
        .overflow_hidden();
    if multiline {
        // A textarea's box height comes from CSS (`min-h-20`) or, failing that,
        // from `rows` via the editor's own min-height. Leaving the wrapper's
        // height AUTO is what lets flexbox stretch it to whichever of those
        // won, and `min-h-full` covers the non-flex parent with a definite
        // height. Text starts at the TOP, as in HTML.
        element = element.min_h_full();
    } else {
        // Fill whatever content box the framework's CSS gave us and centre the
        // line in it, so the whole box is clickable and a tall input does not
        // pin its text to the top.
        element = element.items_center().h_full();
    }

    element = on_state_action(element, state, TextInputState::backspace);
    element = on_state_action(element, state, TextInputState::delete);
    element = on_state_action(element, state, TextInputState::left);
    element = on_state_action(element, state, TextInputState::right);
    element = on_state_action(element, state, TextInputState::select_left);
    element = on_state_action(element, state, TextInputState::select_right);
    element = on_state_action(element, state, TextInputState::select_all);
    element = on_state_action(element, state, TextInputState::home);
    element = on_state_action(element, state, TextInputState::end);
    element = on_state_action(element, state, TextInputState::select_home);
    element = on_state_action(element, state, TextInputState::select_end);
    element = on_state_action(element, state, TextInputState::word_left);
    element = on_state_action(element, state, TextInputState::word_right);
    element = on_state_action(element, state, TextInputState::select_word_left);
    element = on_state_action(element, state, TextInputState::select_word_right);
    element = on_state_action(element, state, TextInputState::delete_word_left);
    element = on_state_action(element, state, TextInputState::delete_word_right);
    element = on_state_action(element, state, TextInputState::delete_to_start);
    element = on_state_action(element, state, TextInputState::delete_to_end);
    // The clipboard commands ask `on_clipboard` first: the DOM fires `copy` /
    // `cut` / `paste` before the default action, and a cancelled one has none.
    let clip = &on_clipboard;
    element = on_clipboard_action(element, state, clip, ClipboardCommand::Copy, TextInputState::copy);
    element = on_clipboard_action(element, state, clip, ClipboardCommand::Cut, TextInputState::cut);
    element = on_clipboard_action(element, state, clip, ClipboardCommand::Paste, TextInputState::paste);
    element = on_state_action(element, state, TextInputState::undo);
    element = on_state_action(element, state, TextInputState::redo);
    element = on_state_action(element, state, TextInputState::clear_selection);
    element = on_state_action(element, state, TextInputState::show_character_palette);
    element = on_state_action(element, state, TextInputState::up);
    element = on_state_action(element, state, TextInputState::down);
    element = on_state_action(element, state, TextInputState::select_up);
    element = on_state_action(element, state, TextInputState::select_down);
    element = on_state_action(element, state, TextInputState::newline);

    // A textarea scrolls its own overflow. The wheel listener has to sit on the
    // wrapper (gpui delivers wheel events to a hitbox, and the wrapper is the
    // one with the box); the caret-following clamp in `set_layout` then keeps
    // whatever the wheel set, because it only moves the offset when the caret
    // leaves the viewport.
    if multiline {
        let scroll_state = state.clone();
        element = element.on_scroll(move |event, window, cx| {
            let delta = event.delta.pixel_delta(px(20.0)).y;
            scroll_state.update(cx, |this, cx| this.scroll_by(-delta, window, cx));
        });
    }

    // `keyup` has no default to cancel, so the plain (post-binding) element
    // listener is enough — unlike `keydown`, which the state entity intercepts
    // *before* gpui resolves key bindings (see `TextInputState::new`).
    if let Some(on_key_up) = on_key_up {
        element = element.on_key_up(move |event, window, cx| on_key_up(event, window, cx));
    }

    let mouse_state = state.clone();
    if !options.inert {
        element = element.on_mouse_down(MouseButton::Left, move |event, window, cx| {
            mouse_state.update(cx, |this, cx| this.on_mouse_down(event, window, cx));
        });
        // The secondary button's own rule (Chromium on Windows): it never
        // starts a drag-selection, and it keeps a selection it lands inside —
        // the context menu is about that selection.
        let menu_state = state.clone();
        element = element.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            menu_state.update(cx, |this, cx| this.on_secondary_mouse_down(event, window, cx));
        });
    }
    element
        .child(TextElement {
            state: state.clone(),
            options,
        })
        .into_any_element()
}

/// Route a gpui action to a method on the state entity. The framework rebuilds
/// the tree every frame from outside any view, so these listeners capture the
/// entity instead of going through `cx.listener`.
fn on_state_action<E, A>(
    element: E,
    state: &Entity<TextInputState>,
    handler: fn(&mut TextInputState, &A, &mut Window, &mut Context<TextInputState>),
) -> E
where
    E: InteractiveElement,
    A: gpui::Action,
{
    let state = state.clone();
    element.on_action(move |action: &A, window, cx| {
        state.update(cx, |this, cx| handler(this, action, window, cx));
    })
}

/// [`on_state_action`] for copy / cut / paste: the hook runs first, outside
/// the state update (it dispatches into JS, which may write the field back),
/// and a `false` from it skips the widget's own handling.
fn on_clipboard_action<E, A>(
    element: E,
    state: &Entity<TextInputState>,
    hook: &Option<OnClipboard>,
    command: ClipboardCommand,
    handler: fn(&mut TextInputState, &A, &mut Window, &mut Context<TextInputState>),
) -> E
where
    E: InteractiveElement,
    A: gpui::Action,
{
    let state = state.clone();
    let hook = hook.clone();
    element.on_action(move |action: &A, window, cx| {
        if let Some(hook) = &hook {
            if !hook(command, window, cx) {
                return;
            }
        }
        state.update(cx, |this, cx| {
            // The hook's handlers may have assigned the field ([`ControlledValue`]).
            this.resync_controlled(cx);
            handler(this, action, window, cx)
        });
    })
}

fn bindings() -> Vec<KeyBinding> {
    let context = Some(KEY_CONTEXT);
    // The platform's "do the thing" modifier, and the one that means "by word",
    // spliced in at compile time: `KeyBinding::new` takes `&'static str`.
    macro_rules! primary {
        ($pre:literal, $key:literal) => {
            if cfg!(target_os = "macos") { concat!($pre, "cmd-", $key) } else { concat!($pre, "ctrl-", $key) }
        };
    }
    macro_rules! word {
        ($pre:literal, $key:literal) => {
            if cfg!(target_os = "macos") { concat!($pre, "alt-", $key) } else { concat!($pre, "ctrl-", $key) }
        };
    }

    let mut bindings = vec![
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("shift-home", SelectHome, context),
        KeyBinding::new("shift-end", SelectEnd, context),
        KeyBinding::new("escape", ClearSelection, context),
    ];

    bindings.extend([
        KeyBinding::new(word!("", "left"), WordLeft, context),
        KeyBinding::new(word!("", "right"), WordRight, context),
        KeyBinding::new(word!("shift-", "left"), SelectWordLeft, context),
        KeyBinding::new(word!("shift-", "right"), SelectWordRight, context),
        KeyBinding::new(word!("", "backspace"), DeleteWordLeft, context),
        KeyBinding::new(word!("", "delete"), DeleteWordRight, context),
    ]);

    bindings.extend([
        KeyBinding::new(primary!("", "a"), SelectAll, context),
        KeyBinding::new(primary!("", "c"), Copy, context),
        KeyBinding::new(primary!("", "x"), Cut, context),
        KeyBinding::new(primary!("", "v"), Paste, context),
        KeyBinding::new(primary!("", "z"), Undo, context),
        KeyBinding::new(primary!("", "y"), Redo, context),
        KeyBinding::new(primary!("shift-", "z"), Redo, context),
    ]);

    // The IBM CUA clipboard keys every Windows and Linux edit control still
    // honours, and so does a browser's text field there.
    if !cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("ctrl-insert", Copy, context),
            KeyBinding::new("shift-delete", Cut, context),
            KeyBinding::new("shift-insert", Paste, context),
        ]);
    }

    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-left", Home, context),
            KeyBinding::new("cmd-right", End, context),
            KeyBinding::new("shift-cmd-left", SelectHome, context),
            KeyBinding::new("shift-cmd-right", SelectEnd, context),
            KeyBinding::new("cmd-backspace", DeleteToStart, context),
            KeyBinding::new("cmd-delete", DeleteToEnd, context),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, context),
        ]);
    }

    // `up`/`down` are shared: in a single-line input a browser moves the caret
    // to the start / end of the value, which is what `TextInputState::up`/
    // `down` do when `multiline` is off.
    bindings.extend([
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
    ]);

    // `enter` is bound in the TEXTAREA context only. A single-line input
    // deliberately leaves Return unbound so the surrounding app can use it
    // (form submit, reka-ui's typeahead), which is also the browser's split:
    // Return in a `<textarea>` inserts a newline and never submits.
    bindings.push(KeyBinding::new(
        "enter",
        Newline,
        Some(TEXTAREA_KEY_CONTEXT),
    ));

    bindings
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Action;

    /// What one keystroke is bound to in the input's context, by action name.
    fn bound(keys: &'static str) -> Option<&'static str> {
        let want = gpui::Keystroke::parse(keys).unwrap();
        bindings()
            .iter()
            .find(|b| b.keystrokes().len() == 1 && *b.keystrokes()[0].inner() == want)
            .map(|b| b.action().name())
    }

    /// The codes are what JS indexes `EDIT_TYPES` with: dense from 1, and only
    /// the IME's type composes (its `beforeinput` cannot be cancelled).
    #[test]
    fn input_types_are_dense_and_only_the_ime_composes() {
        let all = [
            InputType::InsertText,
            InputType::InsertLineBreak,
            InputType::InsertFromPaste,
            InputType::InsertCompositionText,
            InputType::DeleteContentBackward,
            InputType::DeleteContentForward,
            InputType::DeleteWordBackward,
            InputType::DeleteWordForward,
            InputType::DeleteSoftLineBackward,
            InputType::DeleteSoftLineForward,
            InputType::DeleteByCut,
            InputType::HistoryUndo,
            InputType::HistoryRedo,
        ];
        for (i, t) in all.iter().enumerate() {
            assert_eq!(usize::from(t.code()), i + 1, "{t:?}");
            assert_eq!(t.composing(), *t == InputType::InsertCompositionText);
        }
        assert_eq!(InputType::DeleteByCut.name(), "deleteByCut");
        assert_eq!(InputIntent::with(InputType::InsertText, "a").data.as_deref(), Some("a"));
        assert_eq!(InputIntent::bare(InputType::InsertLineBreak).data, None);
    }

    /// Shift+Insert, Ctrl+Insert and Shift+Delete are the same three commands
    /// as Ctrl+V/C/X, so they raise the same ClipboardEvents.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_cua_clipboard_keys_are_the_same_commands() {
        assert_eq!(bound("ctrl-c"), Some(Copy.name()));
        assert_eq!(bound("ctrl-x"), Some(Cut.name()));
        assert_eq!(bound("ctrl-v"), Some(Paste.name()));
        assert_eq!(bound("ctrl-insert"), Some(Copy.name()));
        assert_eq!(bound("shift-delete"), Some(Cut.name()));
        assert_eq!(bound("shift-insert"), Some(Paste.name()));
        // A plain Delete still deletes.
        assert_eq!(bound("delete"), Some(Delete.name()));
    }
}
