//! The interaction state a script's selector query reads.
//!
//! `el.matches(":hover")`, `querySelectorAll(":focus-visible")` and
//! `closest(":focus-within")` filter by the state the element is in NOW, as in
//! a browser. The cascade cannot answer that: it sorts a `:hover` rule into
//! gpui's state ladder and answers "would match when hovered", and gpui picks
//! the rung live at paint time. So a query goes through [`LiveRef`], the same
//! matcher's view of a node with the live state attached, read from what the
//! painter reads:
//!
//! - `:hover` — the hover path ([`crate::pointer::resolve_hover`], resolved
//!   once a frame), which the `:hover` rungs are folded from;
//! - `:active` — the target of the mouse press in progress in the node's
//!   window and its ancestors, from the press to the release (any button, as
//!   gpui's `:active` rung);
//! - `:focus` — the node holding focus in its window (`document.activeElement`);
//! - `:focus-visible` — that node, when its window's last input was the
//!   keyboard (`Window::last_input_was_keyboard`, gpui's `.focus_visible()`
//!   rung) or it is a text field, which Chromium always shows focus on
//!   (`MayTriggerVirtualKeyboard`);
//! - `:focus-within` — that node and its ancestors, up to the root its window
//!   paints.
//!
//! Every window answers for its own nodes: a press or a focus in a pop-out is
//! that window's, and a node is painted by exactly one window.

use std::collections::HashSet;

use gpui::Window;
use vn_css::{Interaction, MatchDom};

use crate::tree::{self, NodeRef, Tree};
use crate::windows;

/// The live state of every window, taken once per query.
#[derive(Default)]
pub(crate) struct LiveStates {
    /// Each window's focused node, and whether it shows focus
    /// (`:focus-visible`).
    focused: Vec<(u32, bool)>,
    /// The focused nodes and their ancestors, each up to its window's painted
    /// root.
    focus_within: HashSet<u32>,
    /// The targets of the presses in progress and their ancestors.
    active: HashSet<u32>,
}

impl LiveStates {
    /// One walk up from each window's focused node and press target: a few
    /// dozen ids, whatever the size of the tree being queried.
    pub(crate) fn take(t: &Tree) -> Self {
        let mut windows_now: Vec<(u32, u32, bool, Option<u32>)> = Vec::new();
        windows::each(|_, w| {
            windows_now.push((w.painted_root, w.focused_node, w.keyboard_input, w.active))
        });
        let mut live = LiveStates::default();
        for (root, focused, keyboard, active) in windows_now {
            if focused != 0 && t.node(focused).is_some() {
                live.focused.push((focused, keyboard || is_text_field(t, focused)));
                chain(t, focused, root, &mut live.focus_within);
            }
            if let Some(target) = active.filter(|&id| t.node(id).is_some()) {
                chain(t, target, root, &mut live.active);
            }
        }
        live
    }

    fn answer(&self, id: u32, state: Interaction) -> bool {
        match state {
            Interaction::Hover => tree::is_hovered(id),
            Interaction::Active => self.active.contains(&id),
            Interaction::Focus => self.focused.iter().any(|&(f, _)| f == id),
            Interaction::FocusVisible => self.focused.iter().any(|&(f, visible)| f == id && visible),
            Interaction::FocusWithin => self.focus_within.contains(&id),
        }
    }
}

/// `id` and its ancestors, up to and including `root` (`0`: the tree's root).
fn chain(t: &Tree, id: u32, root: u32, out: &mut HashSet<u32>) {
    let mut at = id;
    // A tree is a tree, but a corrupt parent chain must not hang a query.
    for _ in 0..4096 {
        out.insert(at);
        if at == root {
            return;
        }
        match t.node(at) {
            Some(n) if n.parent != 0 && n.parent != at => at = n.parent,
            _ => return,
        }
    }
}

/// A text field: Chromium matches `:focus-visible` on it whatever moved the
/// focus there, a click included, because typing goes into it
/// (`HTMLInputElement::MayTriggerVirtualKeyboard`). A `<textarea>`, and an
/// `<input>` of a type the user types into.
pub(crate) fn is_text_field(t: &Tree, id: u32) -> bool {
    let Some(node) = t.node(id) else { return false };
    match t.name(node.tag) {
        "textarea" => true,
        "input" => {
            let kind = t.prop(id, "type").unwrap_or("text").trim().to_ascii_lowercase();
            matches!(
                kind.as_str(),
                "" | "text" | "search" | "url" | "tel" | "email" | "password" | "number"
            )
        }
        _ => false,
    }
}

/// Records which modality this window's last input was, for `:focus-visible`
/// in a query. Called wherever the host is about to run script for a window:
/// before each input dispatch, and once a frame before the JS pump.
pub(crate) fn note_modality(window: &Window) {
    let keyboard = window.last_input_was_keyboard();
    windows::here(window, |w| w.keyboard_input = keyboard);
}

/// The matcher's view of a node for a script's query: [`NodeRef`], with the
/// live interaction state ([`vn_css::MatchDom::LIVE_STATE`]).
#[derive(Clone, Copy)]
pub(crate) struct LiveRef<'t> {
    pub(crate) node: NodeRef<'t>,
    pub(crate) live: &'t LiveStates,
}

impl<'t> LiveRef<'t> {
    pub(crate) fn new(tree: &'t Tree, id: u32, live: &'t LiveStates) -> Self {
        LiveRef { node: NodeRef { tree, id }, live }
    }

    fn wrap(&self, node: NodeRef<'t>) -> Self {
        LiveRef { node, live: self.live }
    }
}

impl<'t> MatchDom for LiveRef<'t> {
    const LIVE_STATE: bool = true;

    fn interaction(&self, state: Interaction) -> Option<bool> {
        Some(self.live.answer(self.node.id, state))
    }
    fn key(&self) -> u64 {
        self.node.key()
    }
    fn parent(&self) -> Option<Self> {
        self.node.parent().map(|n| self.wrap(n))
    }
    fn prev_sibling(&self) -> Option<Self> {
        self.node.prev_sibling().map(|n| self.wrap(n))
    }
    fn next_sibling(&self) -> Option<Self> {
        self.node.next_sibling().map(|n| self.wrap(n))
    }
    fn has_tag(&self, name: &str) -> bool {
        self.node.has_tag(name)
    }
    fn tag_eq(&self, other: &Self) -> bool {
        self.node.tag_eq(&other.node)
    }
    fn has_class(&self, name: &str) -> bool {
        self.node.has_class(name)
    }
    fn tag_name(&self) -> &str {
        self.node.tag_name()
    }
    fn each_class(&self, f: &mut dyn FnMut(&str)) {
        self.node.each_class(f)
    }
    fn has_scope(&self, name: &str) -> bool {
        self.node.has_scope(name)
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.node.attr(name)
    }
    fn is_empty(&self) -> bool {
        self.node.is_empty()
    }
    fn is_root(&self) -> bool {
        self.node.is_root()
    }
    fn each_child(&self, f: &mut dyn FnMut(Self)) {
        let live = self.live;
        self.node.each_child(&mut |n| f(LiveRef { node: n, live }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::{self, QueryMode};

    /// root > nav > (button, input), and root > side > field, `side` painted
    /// by a pop-out (window 2). Window 1 is the primary.
    fn page() -> [u32; 6] {
        let ids = tree::with_tree(|t| {
            let root = t.create_root();
            let (div, button, input) = (t.intern("div"), t.intern("button"), t.intern("input"));
            let nav = t.create_element(div);
            let b = t.create_element(button);
            let i = t.create_element(input);
            let side = t.create_element(div);
            let field = t.create_element(input);
            t.insert(nav, root, 0);
            t.insert(b, nav, 0);
            t.insert(i, nav, 0);
            t.insert(side, root, 0);
            t.insert(field, side, 0);
            let kind = t.intern("type");
            t.set_prop(field, kind, Some("checkbox".into()));
            [root, nav, b, i, side, field]
        });
        windows::set_primary(1);
        windows::set_painted_root(2, ids[4]);
        ids
    }

    fn q(sel: &str, root: u32) -> Vec<u32> {
        dom::query(sel, root, QueryMode::Descendants).unwrap()
    }

    fn is(sel: &str, id: u32) -> bool {
        !dom::query(sel, id, QueryMode::SelfOnly).unwrap().is_empty()
    }

    #[test]
    fn focus_and_focus_visible_follow_the_window_and_its_last_input() {
        let [root, nav, button, input, side, field] = page();
        // Nothing focused: nothing matches, and the query says nothing about it.
        assert!(q(":focus, :focus-visible, :focus-within", root).is_empty());
        // A click focused the button.
        windows::of(1, |w| (w.focused_node, w.keyboard_input) = (button, false));
        assert_eq!(q(":focus", root), vec![button]);
        assert!(q(":focus-visible", root).is_empty(), "a click shows no focus on a button");
        assert_eq!(q(":focus-within", root), vec![nav, button]);
        assert!(is(":focus-within", root), "up to the root the primary window paints");
        assert_eq!(q("button:not(:focus-visible)", root), vec![button]);
        // Tab: the same focus is visible.
        windows::of(1, |w| w.keyboard_input = true);
        assert_eq!(q(":focus-visible", root), vec![button]);
        // A click into a text field shows focus there, as in Chromium.
        windows::of(1, |w| (w.focused_node, w.keyboard_input) = (input, false));
        assert_eq!(q(":focus-visible", root), vec![input]);
        // A pop-out's focus is its own: a checkbox clicked there shows none,
        // and focus within stops at the root the pop-out paints.
        windows::of(1, |w| w.focused_node = 0);
        windows::of(2, |w| (w.focused_node, w.keyboard_input) = (field, false));
        assert_eq!(q(":focus", root), vec![field]);
        assert!(q(":focus-visible", root).is_empty());
        assert_eq!(q(":focus-within", root), vec![side, field]);
        assert!(!is(":focus-within", root));
        windows::of(2, |w| w.keyboard_input = true);
        assert_eq!(q(":focus-visible", root), vec![field]);
    }

    #[test]
    fn hover_and_active_are_the_pointer_path_and_the_press() {
        let [root, nav, button, input, side, field] = page();
        tree::note_hover(nav, true);
        tree::note_hover(button, true);
        assert_eq!(q(":hover", root), vec![nav, button]);
        assert_eq!(q("button:hover, :hover > button", root), vec![button]);
        assert_eq!(q(":hover + input", root), vec![input]);
        assert!(q(":active", root).is_empty());
        // A press on the button, in the primary window: the button and its
        // ancestors are active until the release.
        windows::of(1, |w| w.active = Some(button));
        assert_eq!(q(":active", root), vec![nav, button]);
        assert!(is(":active", root));
        windows::of(1, |w| w.active = None);
        // A press in the pop-out: up to its painted root.
        windows::of(2, |w| w.active = Some(field));
        assert_eq!(q(":active", root), vec![side, field]);
        assert!(!is(":active", root));
    }
}
