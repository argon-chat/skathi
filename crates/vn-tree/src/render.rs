//! Lowers the retained tree into gpui elements. Runs inside the hosting
//! view's `render()` each frame — gpui's element tree is rebuilt per frame by
//! design, so we simply re-emit the whole thing; per-component memoization
//! (`Entity::cached`) is a later optimization.

use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, Empty, SharedString, div, px};

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
            for (k, v) in &node.style {
                d = apply_style(d, t.name(*k), v);
            }
            if node.listeners & (1 << EV_CLICK) != 0 {
                d = d.cursor_pointer().on_click(cx.listener(
                    move |_, _: &ClickEvent, window, cx| {
                        // Legal listener position (fill-1 §2.3): publish and
                        // let the JS handler + Vue's microtask flush run the
                        // nodeOps right here, then repaint this same frame.
                        with_published(window, &mut **cx, Phase::Commit, || {
                            crate::dispatch_event(id, EV_CLICK);
                        });
                        if crate::take_dirty() {
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

/// v0 inline-style mapping — a placeholder until vn-css owns styling.
fn apply_style<E: Styled>(mut e: E, key: &str, v: &str) -> E {
    if v.is_empty() {
        return e; // cleared
    }
    let n = || v.trim_end_matches("px").parse::<f32>().unwrap_or(0.0);
    let color = || gpui::Rgba::try_from(v).ok();
    match key {
        "display" => match v {
            "flex" => e = e.flex(),
            "none" => e = e.hidden(),
            _ => {}
        },
        "flexDirection" => match v {
            "column" => e = e.flex().flex_col(),
            "row" => e = e.flex().flex_row(),
            _ => {}
        },
        "alignItems" if v == "center" => e = e.items_center(),
        "justifyContent" if v == "center" => e = e.justify_center(),
        "flex" if v == "1" => e = e.flex_1(),
        "gap" => e = e.gap(px(n())),
        "padding" => e = e.p(px(n())),
        "margin" => e = e.m(px(n())),
        "width" => e = e.w(px(n())),
        "height" => e = e.h(px(n())),
        "borderRadius" => e = e.rounded(px(n())),
        "fontSize" => e = e.text_size(px(n())),
        "background" | "backgroundColor" => {
            if let Some(c) = color() {
                e = e.bg(c);
            }
        }
        "color" => {
            if let Some(c) = color() {
                e = e.text_color(c);
            }
        }
        _ => {} // v0: unknown properties are dropped silently; vn-css will diagnose
    }
    e
}
