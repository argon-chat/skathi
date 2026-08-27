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
//! * [`TextInputHandlers`] — the per-frame event sinks (`input`, `keydown`,
//!   `keyup`); `keydown` runs *before* the widget acts and can cancel it.
//! * [`text_input`] — builds the element for one frame out of the three.

mod boundaries;
mod element;
mod state;

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, Hsla, InteractiveElement, IntoElement,
    KeyBinding, KeyDownEvent, KeyUpEvent, MouseButton, ParentElement, Pixels, SharedString,
    Styled, Window, actions, div, hsla, px,
};

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

/// Called by [`TextInputState`] whenever the user's editing changed the
/// content; the framework forwards it to JS as an `input`/`change` event.
///
/// Invoked deferred (via [`Window::defer`]), i.e. after the edit that caused it
/// has finished, so it is safe to call back into the same entity — including
/// [`TextInputState::sync_value`] — from inside it.
pub type OnChange = Rc<dyn Fn(&str, &mut Window, &mut App)>;

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

/// The per-frame callback set the framework hands the widget. All optional:
/// the framework only fills in what the JS side actually listens for.
#[derive(Clone, Default)]
pub struct TextInputHandlers {
    /// User editing changed the content (`input`/`change`).
    pub on_change: Option<OnChange>,
    /// Pre-processing hook for keystrokes; see [`OnKeyDown`].
    pub on_key_down: Option<OnKeyDown>,
    pub on_key_up: Option<OnKeyUp>,
}

impl std::fmt::Debug for TextInputHandlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextInputHandlers")
            .field("on_change", &self.on_change.is_some())
            .field("on_key_down", &self.on_key_down.is_some())
            .field("on_key_up", &self.on_key_up.is_some())
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
    /// `<textarea rows>` — the INTRINSIC height, in lines. As in HTML it is
    /// only a minimum: a CSS `height`/`min-height` still wins and content past
    /// it scrolls. Ignored unless `multiline`.
    pub rows: usize,
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
            rows: DEFAULT_ROWS,
            inherit_color: false,
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
    let on_key_up = handlers.on_key_up.clone();
    // The focus handle carries the tab order (gpui reads `tab_index`/`tab_stop`
    // straight off the handle when the element is painted), so it has to be
    // updated before `track_focus` clones it below.
    let focus_handle = state.update(cx, |this, _| {
        this.set_tab_index(options.tab_index);
        this.set_frame_params(handlers, disabled, multiline);
        this.focus_handle()
    });

    let mut element = div()
        .key_context(if multiline { BOTH_KEY_CONTEXTS } else { KEY_CONTEXT })
        .track_focus(&focus_handle)
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
    element = on_state_action(element, state, TextInputState::copy);
    element = on_state_action(element, state, TextInputState::cut);
    element = on_state_action(element, state, TextInputState::paste);
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
        element = element.on_scroll_wheel(move |event, window, cx| {
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
    element
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            mouse_state.update(cx, |this, cx| this.on_mouse_down(event, window, cx));
        })
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

fn bindings() -> Vec<KeyBinding> {
    let context = Some(KEY_CONTEXT);
    // The platform's "do the thing" modifier, and the one that means "by word".
    let (primary, word) = if cfg!(target_os = "macos") {
        ("cmd", "alt")
    } else {
        ("ctrl", "ctrl")
    };

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
        KeyBinding::new(&format!("{word}-left"), WordLeft, context),
        KeyBinding::new(&format!("{word}-right"), WordRight, context),
        KeyBinding::new(&format!("shift-{word}-left"), SelectWordLeft, context),
        KeyBinding::new(&format!("shift-{word}-right"), SelectWordRight, context),
        KeyBinding::new(&format!("{word}-backspace"), DeleteWordLeft, context),
        KeyBinding::new(&format!("{word}-delete"), DeleteWordRight, context),
    ]);

    bindings.extend([
        KeyBinding::new(&format!("{primary}-a"), SelectAll, context),
        KeyBinding::new(&format!("{primary}-c"), Copy, context),
        KeyBinding::new(&format!("{primary}-x"), Cut, context),
        KeyBinding::new(&format!("{primary}-v"), Paste, context),
        KeyBinding::new(&format!("{primary}-z"), Undo, context),
        KeyBinding::new(&format!("{primary}-y"), Redo, context),
        KeyBinding::new(&format!("shift-{primary}-z"), Redo, context),
    ]);

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
