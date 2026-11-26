use std::cell::RefCell;

use gpui::{AnyElement, App, Context, Window, WindowBackgroundAppearance};

use crate::popup::{self, Placement, Slot};
use crate::windows::{self, Rect, WinKey};

#[derive(Clone, Copy)]
pub(crate) struct Menu {
    pub root: u32,
    pub anchor: u32,
    owner: WinKey,
    point: Option<(f32, f32)>,
    pub placement: Placement,
    pub backdrop: WindowBackgroundAppearance,
    pub prepare_window: fn(&Window, &App) -> Option<gpui::Rgba>,
}

thread_local! {
    static MENUS: RefCell<Vec<Menu>> = const { RefCell::new(Vec::new()) };
}

/// Opens a retained DOM subtree as an owned popup at a point or against an element.
pub fn open(
    root: u32,
    anchor: u32,
    point: Option<(f32, f32)>,
    submenu: bool,
    backdrop: WindowBackgroundAppearance,
    prepare_window: fn(&Window, &App) -> Option<gpui::Rgba>,
) {
    let owner = windows::window_of_node(anchor);
    let placement = if submenu {
        Placement::Beside
    } else if point.is_some() {
        Placement::Point
    } else {
        Placement::Below
    };
    MENUS.with(|menus| {
        let mut menus = menus.borrow_mut();
        menus.retain(|m| m.root != root);
        menus.push(Menu {
            root,
            anchor,
            owner,
            point,
            placement,
            backdrop,
            prepare_window,
        });
    });
    crate::set_popped_out(root, true);
    crate::tree::with_tree(|t| {
        t.touch_repaint(root);
        t.touch(anchor);
    });
}

/// Closes a menu and cancels a pending native open for the same subtree.
pub fn close(root: u32, cx: &mut App) -> bool {
    let menu = MENUS.with(|menus| {
        let mut menus = menus.borrow_mut();
        let i = menus.iter().position(|m| m.root == root)?;
        Some(menus.remove(i))
    });
    if let Some(menu) = menu {
        popup::close(Slot::Menu(root), menu.anchor, cx);
        crate::set_popped_out(root, false);
        crate::tree::with_tree(|t| {
            t.touch_repaint(root);
            t.touch(menu.anchor);
        });
        return true;
    }
    false
}

pub(crate) fn dismiss(root: u32, cx: &mut App) {
    if !close(root, cx) {
        return;
    }
    sk_runtime::with_published_app(cx, || {
        sk_runtime::JsRuntime::try_with(|rt| {
            let ctx = rt.ctx();
            if let Ok(callback) = ctx.global_get("__sk_context_menu_dismiss") {
                use sk_runtime::redjs::api::Value;
                let _ = ctx.call(
                    callback,
                    Value::undefined(),
                    &[Value::number(f64::from(root))],
                );
                ctx.drain_microtasks();
            }
        });
    });
}

pub(crate) fn deactivate(window: &Window, cx: &mut App) {
    let owner = windows::key(window);
    let root = MENUS.with(|menus| {
        menus
            .borrow()
            .iter()
            .find(|m| m.owner == owner)
            .map(|m| m.root)
    });
    if let Some(root) = root {
        cx.defer(move |cx| dismiss(root, cx));
    }
}

pub(crate) fn layer<V: 'static>(
    tree: &crate::tree::Tree,
    window: &mut Window,
    cx: &mut Context<V>,
    mut build: impl FnMut(u32, &mut Window, &mut Context<V>) -> AnyElement,
) -> Vec<AnyElement> {
    let owner = windows::key(window);
    let menus: Vec<_> = MENUS.with(|menus| {
        menus
            .borrow()
            .iter()
            .filter(|m| m.owner == owner)
            .copied()
            .collect()
    });
    menus
        .into_iter()
        .filter_map(|menu| {
            if tree.node(menu.root).is_none() || tree.node(menu.anchor).is_none() {
                cx.defer(move |cx| dismiss(menu.root, cx));
                return None;
            }
            let child = build(menu.root, window, cx);
            let anchor: Option<Rect> = menu.point.map(|(x, y)| (x, y, 0.0, 0.0));
            Some(popup::menu_element(menu, anchor, child))
        })
        .collect()
}
