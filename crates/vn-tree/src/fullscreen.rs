//! Fullscreen changes the painted root without moving or cloning DOM nodes.
use crate::tree::{self, Tree};
use std::cell::Cell;

thread_local! { static ELEMENT: Cell<u32> = const { Cell::new(0) }; }

pub(crate) fn set(id: u32) -> bool {
    tree::with_tree(|tree| {
        if id != 0
            && (!tree
                .node(id)
                .is_some_and(|node| node.kind == crate::tree::Kind::Element)
                || !connected(tree, id))
        {
            return false;
        }
        let previous = ELEMENT.with(|element| element.replace(id));
        let key = tree.intern(":fullscreen");
        if previous != 0 {
            tree.set_prop(previous, key, None);
        }
        if id != 0 {
            tree.set_prop(id, key, Some(String::new()));
        }
        tree.mark_dirty();
        true
    })
}

pub(crate) fn current(tree: &Tree) -> Option<u32> {
    let id = ELEMENT.with(Cell::get);
    (id != 0 && connected(tree, id) && tree.prop(id, ":fullscreen").is_some()).then_some(id)
}

pub(crate) fn is_active(id: u32) -> bool {
    ELEMENT.with(Cell::get) == id && id != 0 && tree::read_tree(|tree| current(tree) == Some(id))
}

fn connected(tree: &Tree, mut id: u32) -> bool {
    for _ in 0..4096 {
        if id == tree.root && id != 0 {
            return true;
        }
        let Some(node) = tree.node(id) else {
            return false;
        };
        id = node.parent;
    }
    false
}

pub(crate) fn apply_style(style: &mut gpui::StyleRefinement) {
    let full = gpui::Length::Definite(gpui::relative(1.));
    style.position = Some(gpui::Position::Relative);
    style.size.width = Some(full);
    style.size.height = Some(full);
    style.min_size.width = Some(gpui::px(0.).into());
    style.min_size.height = Some(gpui::px(0.).into());
    style.max_size.width = Some(gpui::Length::Auto);
    style.max_size.height = Some(gpui::Length::Auto);
    style.aspect_ratio = None;
    style.flex_grow = Some(0.);
    style.flex_shrink = Some(0.);
    style.flex_basis = Some(gpui::Length::Auto);
    style.vn_transform = None;
    style.margin.top = Some(gpui::px(0.).into());
    style.margin.right = Some(gpui::px(0.).into());
    style.margin.bottom = Some(gpui::px(0.).into());
    style.margin.left = Some(gpui::px(0.).into());
    style.inset.top = Some(gpui::Length::Auto);
    style.inset.right = Some(gpui::Length::Auto);
    style.inset.bottom = Some(gpui::Length::Auto);
    style.inset.left = Some(gpui::Length::Auto);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_must_be_connected_and_is_released_when_removed() {
        let (root, connected, detached) = tree::with_tree(|tree| {
            let root = tree.create_root();
            let tag = tree.intern("div");
            let connected = tree.create_element(tag);
            tree.insert(connected, root, 0);
            let detached = tree.create_element(tag);
            (root, connected, detached)
        });
        assert!(!set(detached));
        assert!(set(connected));
        tree::read_tree(|tree| {
            assert_eq!(current(tree), Some(connected));
            assert!(tree.prop(connected, ":fullscreen").is_some());
        });
        tree::with_tree(|tree| tree.remove(connected));
        tree::read_tree(|tree| assert_eq!(current(tree), None));
        assert!(!is_active(connected));
        assert!(set(0));
        tree::with_tree(|tree| tree.remove(root));
    }
}
