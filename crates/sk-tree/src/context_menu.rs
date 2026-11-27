use std::cell::RefCell;

use gpui::{AnyElement, App, Context, Window, WindowBackgroundAppearance};

use crate::popup::{self, Placement, Slot};
use crate::windows::{self, Rect, WinKey};

pub(crate) mod aim;

#[derive(Clone, Copy)]
pub(crate) struct Menu {
    pub root: u32,
    pub anchor: u32,
    owner: WinKey,
    point: Option<(f32, f32)>,
    bounds_in_owner: Option<Rect>,
    parent: u32,
    aim: Option<aim::Aim>,
    pub debug_aim: bool,
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
    parent: u32,
    debug_aim: bool,
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
            bounds_in_owner: None,
            parent,
            aim: None,
            debug_aim,
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

pub(crate) fn placed(root: u32, bounds: Rect, cx: &mut App) {
    MENUS.with_borrow_mut(|menus| {
        let Some(m) = menus.iter_mut().find(|m| m.root == root) else {
            return;
        };
        if m.bounds_in_owner == Some(bounds) {
            return;
        }
        m.bounds_in_owner = Some(bounds);
        if m.debug_aim && m.aim.is_some() {
            aim::reposition(m.parent, root, aim::Zone::new(m.aim, Some(bounds)));
            refresh_trace(m.owner, cx);
        }
    });
}

fn refresh_trace(owner: WinKey, cx: &mut App) {
    cx.defer(move |cx| {
        if let Some(handle) = popup::handle_of(owner, cx) {
            let _ = handle.update(cx, |_, window, _| {
                // A submenu can finish opening after its idle parent has already painted.
                window.on_next_frame(|window, _| window.refresh());
            });
        }
    });
}

#[derive(PartialEq, Eq)]
pub enum PointerAt {
    Trigger,
    Row,
    Gap,
    Reset,
}

/// Updates the submenu approach and returns whether the crossed row should yield.
pub fn pointer(root: u32, p: (f32, f32), at: PointerAt) -> bool {
    MENUS.with_borrow_mut(|menus| {
        let Some(m) = menus
            .iter_mut()
            .find(|m| m.root == root && m.placement == Placement::Beside)
        else {
            return false;
        };
        if at == PointerAt::Reset {
            m.aim = None;
            return false;
        }
        if at == PointerAt::Trigger {
            m.aim = Some(aim::Aim::on_trigger(m.aim, p));
            if !m.debug_aim {
                return false;
            }
        }
        let zone = aim::Zone::new(m.aim, m.bounds_in_owner);
        let hit = zone.hit(p);
        if at != PointerAt::Trigger {
            if let (Some(a), Some(d)) = (&mut m.aim, hit) {
                a.advance(d, p.0);
            }
        }
        if m.debug_aim {
            let state = if at == PointerAt::Trigger {
                aim::State::Tracking
            } else if hit.is_some() {
                aim::State::Held
            } else if at == PointerAt::Row {
                aim::State::Released
            } else {
                aim::State::Tracking
            };
            aim::record(m.parent, root, zone, state, p);
            sk_runtime::with_app(|cx| refresh_trace(m.owner, cx));
        }
        hit.is_some()
    })
}

/// Closes a menu and cancels a pending native open for the same subtree.
pub fn close(root: u32, cx: &mut App) -> bool {
    let menu = MENUS.with(|menus| {
        let mut menus = menus.borrow_mut();
        let i = menus.iter().position(|m| m.root == root)?;
        Some(menus.remove(i))
    });
    if let Some(menu) = menu {
        if menu.debug_aim {
            aim::clear(root);
        }
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
