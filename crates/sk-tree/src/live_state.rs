//! The interaction state a script's selector query reads.
//!
//! `el.matches(":hover")` filters by the state the element is in NOW, which the cascade cannot answer
//! (it sorts `:hover` rules into gpui's state ladder; gpui picks the rung at paint time). A query goes
//! through [`LiveRef`], the matcher's view of a node with live state read from what the painter
//! reads. Every window answers for its own nodes; a `:focus-visible` text field always shows focus,
//! as in Chromium.

use std::cell::Cell;
use std::collections::HashSet;

use gpui::Window;
use sk_css::{Interaction, MatchDom};

use crate::tree::{self, NodeRef, Tree};
use crate::windows;

#[derive(Default)]
pub(crate) struct LiveStates {
    focused: Vec<(u32, bool)>,
    focus_within: HashSet<u32>,
    active: HashSet<u32>,
}

impl LiveStates {
    pub(crate) fn take(t: &Tree) -> Self {
        let mut windows_now: Vec<(u32, u32, bool, Option<u32>)> = Vec::new();
        windows::each(|_, w| {
            windows_now.push((w.painted_root, w.focused_node, w.keyboard_input, w.active))
        });
        let mut live = LiveStates::default();
        for (root, focused, keyboard, active) in windows_now {
            if focused != 0 && t.node(focused).is_some() {
                live.focused
                    .push((focused, keyboard || is_text_field(t, focused)));
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
            Interaction::FocusVisible => {
                self.focused.iter().any(|&(f, visible)| f == id && visible)
            }
            Interaction::FocusWithin => self.focus_within.contains(&id),
        }
    }
}

thread_local! {
    /// The focused node and modality gpui paints this frame, read when a window's build starts: the
    /// window's slot catches up only in gpui's focus observers, which run after the draw.
    static DRAWN_FOCUS: Cell<(u32, bool)> = const { Cell::new((0, false)) };
}

pub(crate) fn begin_draw(focused: Option<u32>, window: &Window) {
    DRAWN_FOCUS.with(|c| c.set((focused.unwrap_or(0), window.last_input_was_keyboard())));
}

/// Whether `anchor` is in a focus state in the frame being drawn, for ancestor-focus rules.
pub(crate) fn drawn_focus_holds(t: &Tree, anchor: u32, state: Interaction) -> bool {
    let (focused, keyboard) = DRAWN_FOCUS.with(Cell::get);
    focus_state(t, focused, keyboard, anchor, state)
}

/// [`drawn_focus_holds`] between frames, from the windows' slots.
pub(crate) fn focus_holds(t: &Tree, anchor: u32, state: Interaction) -> bool {
    let mut focused: Vec<(u32, bool)> = Vec::new();
    windows::each(|_, w| {
        if w.focused_node != 0 {
            focused.push((w.focused_node, w.keyboard_input));
        }
    });
    focused
        .into_iter()
        .any(|(f, keyboard)| focus_state(t, f, keyboard, anchor, state))
}

fn focus_state(t: &Tree, focused: u32, keyboard: bool, anchor: u32, state: Interaction) -> bool {
    if focused == 0 {
        return false;
    }
    match state {
        Interaction::Focus => focused == anchor,
        Interaction::FocusVisible => focused == anchor && (keyboard || is_text_field(t, focused)),
        Interaction::FocusWithin => {
            let mut at = focused;
            for _ in 0..4096 {
                if at == anchor {
                    return true;
                }
                match t.node(at) {
                    Some(n) if n.parent != 0 && n.parent != at => at = n.parent,
                    _ => return false,
                }
            }
            false
        }
        Interaction::Hover | Interaction::Active => false,
    }
}

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

/// A text field: Chromium matches `:focus-visible` on it whatever moved the focus there, a click
/// included, because typing goes into it (`HTMLInputElement::MayTriggerVirtualKeyboard`).
pub(crate) fn is_text_field(t: &Tree, id: u32) -> bool {
    let Some(node) = t.node(id) else { return false };
    match t.name(node.tag) {
        "textarea" => true,
        "input" => {
            let kind = t
                .prop(id, "type")
                .unwrap_or("text")
                .trim()
                .to_ascii_lowercase();
            matches!(
                kind.as_str(),
                "" | "text" | "search" | "url" | "tel" | "email" | "password" | "number"
            )
        }
        _ => false,
    }
}

pub(crate) fn note_modality(window: &Window) {
    let keyboard = window.last_input_was_keyboard();
    windows::here(window, |w| w.keyboard_input = keyboard);
}

#[derive(Clone, Copy)]
pub(crate) struct LiveRef<'t> {
    pub(crate) node: NodeRef<'t>,
    pub(crate) live: &'t LiveStates,
}

impl<'t> LiveRef<'t> {
    pub(crate) fn new(tree: &'t Tree, id: u32, live: &'t LiveStates) -> Self {
        LiveRef {
            node: NodeRef { tree, id },
            live,
        }
    }

    fn wrap(&self, node: NodeRef<'t>) -> Self {
        LiveRef {
            node,
            live: self.live,
        }
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
        assert!(q(":focus, :focus-visible, :focus-within", root).is_empty());
        windows::of(1, |w| (w.focused_node, w.keyboard_input) = (button, false));
        assert_eq!(q(":focus", root), vec![button]);
        assert!(
            q(":focus-visible", root).is_empty(),
            "a click shows no focus on a button"
        );
        assert_eq!(q(":focus-within", root), vec![nav, button]);
        assert!(
            is(":focus-within", root),
            "up to the root the primary window paints"
        );
        assert_eq!(q("button:not(:focus-visible)", root), vec![button]);
        windows::of(1, |w| w.keyboard_input = true);
        assert_eq!(q(":focus-visible", root), vec![button]);
        windows::of(1, |w| (w.focused_node, w.keyboard_input) = (input, false));
        assert_eq!(q(":focus-visible", root), vec![input]);
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
        windows::of(1, |w| w.active = Some(button));
        assert_eq!(q(":active", root), vec![nav, button]);
        assert!(is(":active", root));
        windows::of(1, |w| w.active = None);
        windows::of(2, |w| w.active = Some(field));
        assert_eq!(q(":active", root), vec![side, field]);
        assert!(!is(":active", root));
    }
}
