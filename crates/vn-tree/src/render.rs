//! Lowers the retained tree into gpui elements. Runs inside the hosting
//! view's `render()` each frame — gpui's element tree is rebuilt per frame by
//! design, so we simply re-emit the whole thing; per-component memoization
//! (`Entity::cached`) is a later optimization.

use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, Empty, SharedString, div};

use crate::tree::{self, EV_CLICK, Kind, Tree};
use vn_runtime::{Phase, with_published};

/// Builds the current tree root (or nothing) for the hosting view `V`.
pub fn build_root<V: 'static>(cx: &mut Context<V>) -> AnyElement {
    tree::read_tree(|t| {
        if t.root == 0 {
            Empty.into_any_element()
        } else {
            build_node(t, t.root, cx)
        }
    })
}

fn build_node<V: 'static>(t: &Tree, id: u32, cx: &mut Context<V>) -> AnyElement {
    let Some(node) = t.node(id) else {
        return Empty.into_any_element();
    };
    match node.kind {
        Kind::Text => SharedString::from(node.text.clone()).into_any_element(),
        Kind::Anchor => Empty.into_any_element(),
        Kind::Element => {
            // Every mirrored node gets a stable ElementId — hover/active/
            // scroll state in gpui is keyed by it.
            let mut d = div().id(id as usize);
            if let Some(cs) = &node.computed {
                // Base wholesale; state refinements go into gpui's own
                // interactivity ladder (the spike-proven installation trick —
                // the FnOnce builders run eagerly, only the result is stored).
                *d.style() = (*cs.base).clone();
                if let Some(h) = cs.hover.clone() {
                    d = d.hover(move |_| (*h).clone());
                }
                if let Some(a) = cs.active.clone() {
                    d = d.active(move |_| (*a).clone());
                }
            }
            if node.listeners & (1 << EV_CLICK) != 0 {
                d = d.on_click(cx.listener(
                    move |_, _: &ClickEvent, window, cx| {
                        // Legal listener position (fill-1 §2.3): publish and
                        // let the JS handler + Vue's microtask flush run the
                        // nodeOps right here, then repaint this same frame.
                        with_published(window, &mut **cx, Phase::Commit, || {
                            crate::dispatch_event(id, EV_CLICK);
                        });
                        if crate::flush_changes() {
                            cx.notify();
                        }
                    },
                ));
            }
            if !node.text.is_empty() {
                d.child(node.text.clone()).into_any_element()
            } else {
                d.children(node.children.iter().map(|&c| build_node(t, c, cx)))
                    .into_any_element()
            }
        }
    }
}
