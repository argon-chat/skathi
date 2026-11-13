//! Native gpui widgets too latency-sensitive to live in JS: [`text_input`] (editor with IME,
//! selection, clipboard, undo/redo, caret) and scrollbars.
//!
//! A component, not a view: the framework owns one [`Entity<TextInputState>`](gpui::Entity) per
//! input node and rebuilds the element tree every frame.
//! ```ignore
//! // once, at startup
//! vn_elements::init(cx);
//!
//! // per input node, once — kept in the framework's side map
//! let state = cx.new(|cx| TextInputState::new(window, cx));
//!
//! // every frame
//! state.update(cx, |state, cx| state.sync_value(&node.value, cx));
//! let element = text_input(&state, options, handlers, cx);
//! ```

pub mod scrollbar;
pub mod tab_nav;
pub mod text_input;

pub use scrollbar::{ChainSpan, Overscroll, Scrollbar, ScrollbarStyle, undo_wheel_scroll};
pub use tab_nav::{FocusNext, FocusPrev};
pub use text_input::{
    ClipboardCommand, CompositionPhase, ControlledValue, DEFAULT_ROWS, InputAttributes, InputIntent,
    InputType, KEY_CONTEXT, MappedConfiguration, OnBeforeInput, OnChange, OnClipboard,
    OnComposition, OnKeyDown, OnKeyUp, TEXTAREA_KEY_CONTEXT, TextInputHandlers, TextInputOptions,
    TextInputState, configuration, text_input,
};

/// One-time setup (key bindings, actions, `Tab`/`Shift-Tab` focus traversal); call before opening a window.
pub fn init(cx: &mut gpui::App) {
    text_input::init(cx);
    tab_nav::init(cx);
}
