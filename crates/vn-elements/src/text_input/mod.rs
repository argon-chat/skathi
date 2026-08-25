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
//!   (placeholder, colours, font size, disabled).
//! * [`text_input`] — builds the element for one frame out of the two.

mod boundaries;
mod element;
mod state;

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, Hsla, InteractiveElement, IntoElement,
    KeyBinding, MouseButton, ParentElement, Pixels, SharedString, Styled, Window, actions, div,
    hsla, px,
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
    ]
);

/// The `key_context` the input's key bindings are scoped to. Exposed so an
/// embedder can write its own bindings against the same context.
pub const KEY_CONTEXT: &str = "VnTextInput";

/// Called by [`TextInputState`] whenever the user's editing changed the
/// content; the framework forwards it to JS as an `input`/`change` event.
///
/// Invoked deferred (via [`Window::defer`]), i.e. after the edit that caused it
/// has finished, so it is safe to call back into the same entity — including
/// [`TextInputState::sync_value`] — from inside it.
pub type OnChange = Rc<dyn Fn(&str, &mut Window, &mut App)>;

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
/// styling for this frame, `on_change` the sink for user edits.
pub fn text_input(
    state: &Entity<TextInputState>,
    options: TextInputOptions,
    on_change: OnChange,
    cx: &mut App,
) -> AnyElement {
    let disabled = options.disabled;
    let focus_handle = state.update(cx, |this, _| {
        this.set_frame_params(on_change, disabled);
        this.focus_handle()
    });

    let mut element = div()
        .key_context(KEY_CONTEXT)
        .track_focus(&focus_handle)
        .cursor(if disabled {
            CursorStyle::Arrow
        } else {
            CursorStyle::IBeam
        })
        // Fill whatever content box the framework's CSS gave us and centre the
        // line in it, so the whole box is clickable and a tall input does not
        // pin its text to the top.
        .flex()
        .items_center()
        .w_full()
        .h_full()
        .min_w_0()
        .overflow_hidden();

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

    // Deliberately unbound: `enter`. A single-line input ignores it, and
    // leaving it unbound lets the surrounding app use it (form submit, ...).
    bindings
}
