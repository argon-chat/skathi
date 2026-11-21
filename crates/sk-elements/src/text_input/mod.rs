//! A reusable text input for gpui (`<input>` and `<textarea>`).
//!
//! Adapted from gpui-vue (`crates/gpui-vue-core/src/custom_elements/input.rs`, Apache-2.0), itself
//! derived from [remorses/gpuix](https://github.com/remorses/gpuix) @
//! `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69` (Apache-2.0) and [zeronsh/comet](https://github.com/zeronsh/comet)
//! (MIT); the caret blink is ported from Comet's `crates/ui/src/composer.rs`. API usage follows
//! gpui's own `crates/gpui/examples/input.rs` (Apache-2.0, zed-industries/zed).
//!
//! The framework keeps one [`Entity<TextInputState>`] per node and rebuilds the element tree every
//! frame, so [`TextInputState`] holds what survives a frame, [`TextInputOptions`] what CSS
//! recomputes per frame, and [`TextInputHandlers`] the per-frame event sinks. [`text_input`] builds
//! the element.

mod boundaries;
pub mod config;
mod element;
mod state;

use std::rc::Rc;

use gpui::{
    actions, div, hsla, px, AnyElement, App, Context, CursorStyle, Entity, Hsla,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, KeyUpEvent, MouseButton,
    ParentElement, Pixels, SharedString, Styled, TextInputConfiguration, Window,
};

pub use config::{configuration, InputAttributes, MappedConfiguration};
pub use state::TextInputState;

use element::TextElement;

actions!(
    sk_text_input,
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
        Down,
        Newline,
        SelectDown,
        SelectUp,
        Up,
    ]
);

pub const KEY_CONTEXT: &str = "SkTextInput";

/// Extra `key_context` of a multi-line field. gpui parses a context as a set, so a textarea
/// inherits every editing binding and adds `enter`, `up`, `down`.
pub const TEXTAREA_KEY_CONTEXT: &str = "SkTextArea";
const BOTH_KEY_CONTEXTS: &str = "SkTextInput SkTextArea";

pub const DEFAULT_ROWS: usize = 2;

/// What a user edit does: the DOM's `InputEvent.inputType`. The discriminant is the code carried to
/// JS, where `EDIT_TYPES` in `packages/runtime/src/runtime/event-names.ts` names it (`test/protocol-codes.test.ts`
/// parses this enum).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum InputType {
    InsertText = 1,
    InsertLineBreak = 2,
    InsertFromPaste = 3,
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

    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn composing(self) -> bool {
        matches!(self, InputType::InsertCompositionText)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputIntent {
    pub input_type: InputType,
    pub data: Option<String>,
}

impl InputIntent {
    pub fn bare(input_type: InputType) -> Self {
        Self {
            input_type,
            data: None,
        }
    }

    pub fn with(input_type: InputType, data: &str) -> Self {
        Self {
            input_type,
            data: Some(data.to_owned()),
        }
    }
}

/// Called when user editing changed the content. Deferred ([`Window::defer`]), so it may call back
/// into the entity, including [`TextInputState::sync_value`].
pub type OnChange = Rc<dyn Fn(&str, &InputIntent, &mut Window, &mut App)>;

/// Fired before every user edit (DOM `beforeinput`), after `keydown`; `false` cancels it. Not called
/// when the edit would do nothing, and IME edits ([`InputType::composing`]) cannot be cancelled.
/// Field assignments made by its handlers are read back ([`ControlledValue`]) before the edit.
pub type OnBeforeInput = Rc<dyn Fn(&InputIntent, &mut Window, &mut App) -> bool>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositionPhase {
    Start,
    Update,
    End,
}

/// The DOM's `compositionstart` / `compositionupdate` / `compositionend` with their `data`, in
/// Chromium's order: start, `beforeinput`, update, the edit, `input`, then `compositionend`. `End`
/// is deferred; every `Start` gets its `End`, including on cancel, focus loss or a value assignment.
pub type OnComposition = Rc<dyn Fn(CompositionPhase, &str, &mut Window, &mut App)>;

/// Fired for every keystroke before the widget acts on it (DOM `keydown`); `false` is `preventDefault()`.
pub type OnKeyDown = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App) -> bool>;

pub type OnKeyUp = Rc<dyn Fn(&KeyUpEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardCommand {
    Copy,
    Cut,
    Paste,
}

/// Fired on copy, cut or paste before the widget touches the clipboard or content; `false` cancels
/// (writing a cancelled copy's own data is the hook's job). Called outside any update of the state.
pub type OnClipboard = Rc<dyn Fn(ClipboardCommand, &mut Window, &mut App) -> bool>;

/// The framework's controlled value as it stands now: the string and its write counter, or `None`.
/// Re-read after a keydown or clipboard hook lets the event through: its handlers may have assigned
/// the field and the default action must edit what they wrote.
pub type ControlledValue = Rc<dyn Fn(&App) -> Option<(String, u32)>>;

#[derive(Clone, Default)]
pub struct TextInputHandlers {
    pub on_change: Option<OnChange>,
    pub on_before_input: Option<OnBeforeInput>,
    pub on_composition: Option<OnComposition>,
    pub on_key_down: Option<OnKeyDown>,
    pub on_key_up: Option<OnKeyUp>,
    pub on_clipboard: Option<OnClipboard>,
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
    pub fn with_change(on_change: OnChange) -> Self {
        Self {
            on_change: Some(on_change),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextInputOptions {
    pub placeholder: SharedString,
    pub text_color: Hsla,
    pub placeholder_color: Hsla,
    pub selection_color: Hsla,
    pub caret_color: Hsla,
    pub font_size: Pixels,
    /// Read-only: no typing, paste, cut, undo/redo, IME or caret; focus, selection and copy work.
    pub disabled: bool,
    pub tab_index: Option<isize>,
    pub multiline: bool,
    pub size_to_content: bool,
    /// The cascade gave no `color`: paint in the inherited one (sk-css folds no inherited properties).
    pub inherit_color: bool,
    /// CSS `text-align`, applied as a content offset (`TextInputState::align_offset`).
    pub text_align: gpui::TextAlign,
    pub rows: usize,
    pub input_config: TextInputConfiguration,
    /// CSS `pointer-events: none` is in force: no press, no automatic focus transfer. Dropping the
    /// tracked focus handle costs the tab stop but keeps hidden overlaid panels from swallowing input.
    pub inert: bool,
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
            size_to_content: false,
            text_align: gpui::TextAlign::Left,
            rows: DEFAULT_ROWS,
            inherit_color: false,
            input_config: TextInputConfiguration::default(),
            inert: false,
            masked: false,
        }
    }
}

pub fn init(cx: &mut App) {
    cx.bind_keys(bindings());
}

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
    // The focus handle carries the tab order, so update it before `track_focus` clones it.
    let focus_handle = state.update(cx, |this, _| {
        this.set_tab_index(options.tab_index);
        this.set_frame_params(handlers, disabled, multiline, input_config);
        this.set_masked(options.masked);
        this.focus_handle()
    });

    let mut element = div().key_context(if multiline {
        BOTH_KEY_CONTEXTS
    } else {
        KEY_CONTEXT
    });
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
    if options.size_to_content && multiline {
        element = element.flex_grow(1.0).min_h(px(0.0)).items_stretch();
    } else if multiline {
        // Height comes from CSS (`min-h-20`) or `rows`; an AUTO wrapper height lets flexbox stretch it.
        element = element.min_h_full();
    } else {
        // Fill the content box and centre the line so the whole box is clickable.
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
    let clip = &on_clipboard;
    element = on_clipboard_action(
        element,
        state,
        clip,
        ClipboardCommand::Copy,
        TextInputState::copy,
    );
    element = on_clipboard_action(
        element,
        state,
        clip,
        ClipboardCommand::Cut,
        TextInputState::cut,
    );
    element = on_clipboard_action(
        element,
        state,
        clip,
        ClipboardCommand::Paste,
        TextInputState::paste,
    );
    element = on_state_action(element, state, TextInputState::undo);
    element = on_state_action(element, state, TextInputState::redo);
    element = on_state_action(element, state, TextInputState::clear_selection);
    element = on_state_action(element, state, TextInputState::show_character_palette);
    element = on_state_action(element, state, TextInputState::up);
    element = on_state_action(element, state, TextInputState::down);
    element = on_state_action(element, state, TextInputState::select_up);
    element = on_state_action(element, state, TextInputState::select_down);
    element = on_state_action(element, state, TextInputState::newline);

    // The wheel listener sits on the wrapper (gpui delivers wheel events to a hitbox).
    if multiline {
        let scroll_state = state.clone();
        element = element.on_scroll(move |event, window, cx| {
            let delta = event.delta.pixel_delta(px(20.0)).y;
            scroll_state.update(cx, |this, cx| this.scroll_by(-delta, window, cx));
        });
    }

    if let Some(on_key_up) = on_key_up {
        element = element.on_key_up(move |event, window, cx| on_key_up(event, window, cx));
    }

    let mouse_state = state.clone();
    if !options.inert {
        element = element.on_mouse_down(MouseButton::Left, move |event, window, cx| {
            mouse_state.update(cx, |this, cx| this.on_mouse_down(event, window, cx));
        });
        let menu_state = state.clone();
        element = element.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            menu_state.update(cx, |this, cx| {
                this.on_secondary_mouse_down(event, window, cx)
            });
        });
    }
    element
        .child(TextElement {
            state: state.clone(),
            options,
        })
        .into_any_element()
}

/// Routes a gpui action to a state-entity method; the tree is rebuilt outside any view.
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

/// [`on_state_action`] for copy / cut / paste: the hook runs first, outside the state update (it
/// dispatches into JS), and `false` skips the widget's handling.
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
            this.resync_controlled(cx);
            handler(this, action, window, cx)
        });
    })
}

fn bindings() -> Vec<KeyBinding> {
    let context = Some(KEY_CONTEXT);
    // The platform's modifier for "by word", spliced in at compile time: `KeyBinding::new` takes `&'static str`.
    macro_rules! primary {
        ($pre:literal, $key:literal) => {
            if cfg!(target_os = "macos") {
                concat!($pre, "cmd-", $key)
            } else {
                concat!($pre, "ctrl-", $key)
            }
        };
    }
    macro_rules! word {
        ($pre:literal, $key:literal) => {
            if cfg!(target_os = "macos") {
                concat!($pre, "alt-", $key)
            } else {
                concat!($pre, "ctrl-", $key)
            }
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

    // `up`/`down` are shared: single-line, they move the caret to the start/end, as in a browser.
    bindings.extend([
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
    ]);

    // `enter` is bound in the TEXTAREA context only, so a single-line input leaves Return to the app.
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

    fn bound(keys: &'static str) -> Option<&'static str> {
        let want = gpui::Keystroke::parse(keys).unwrap();
        bindings()
            .iter()
            .find(|b| b.keystrokes().len() == 1 && *b.keystrokes()[0].inner() == want)
            .map(|b| b.action().name())
    }

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
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_cua_clipboard_keys_are_the_same_commands() {
        assert_eq!(bound("ctrl-c"), Some(Copy.name()));
        assert_eq!(bound("ctrl-x"), Some(Cut.name()));
        assert_eq!(bound("ctrl-v"), Some(Paste.name()));
        assert_eq!(bound("ctrl-insert"), Some(Copy.name()));
        assert_eq!(bound("shift-delete"), Some(Cut.name()));
        assert_eq!(bound("shift-insert"), Some(Paste.name()));
        assert_eq!(bound("delete"), Some(Delete.name()));
    }
}
