//! The used `color-scheme` of the controls Skathi draws.

use gpui::Window;

use crate::tree::Tree;
use crate::windows;

/// `Some(dark)` when the element's `color-scheme` names light or dark; `None` under `normal`, where
/// each control keeps its own default.
pub(crate) fn declared(t: &Tree, id: u32, window: &Window) -> Option<bool> {
    let vars = &t.node(id)?.computed.as_ref()?.vars;
    let env = windows::env_of(windows::key(window)).unwrap_or_else(crate::env);
    sk_css::color_scheme(vars, &env)
}
