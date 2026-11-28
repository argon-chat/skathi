//! Popups (the color picker, a select's list): a borderless window of their own, placed under the
//! anchor's box on its monitor's work area, so they may extend past the app's window (on Wayland
//! the compositor places it against the box); drawn over the window's content, kept inside it, when
//! that window cannot be opened. Under its anchor, or above when it does not fit and fits better
//! there, kept inside sideways (gpui's `anchored` flips around one point, a popup around a box).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::prelude::*;
use gpui::{div, px, AnyElement, App, Bounds, Hsla, Pixels, Window};

use crate::windows::{self, Rect, WinKey};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[path = "popup/linux.rs"]
mod plat;
#[cfg(target_os = "macos")]
#[path = "popup/mac.rs"]
mod plat;
#[cfg(windows)]
#[path = "popup/win.rs"]
mod plat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Picker,
    Select,
    Menu(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Below,
    Point,
    Beside,
}

pub(crate) type Build = Rc<dyn Fn() -> AnyElement>;

pub(crate) struct Popup {
    pub slot: Slot,
    /// The node whose painted box the popup hangs from.
    pub anchor: u32,
    /// The box to use when the anchor paints nothing (`display: none`).
    pub fallback: Rect,
    pub gap: f32,
    /// Transparent room around a popup window's content, for its shadow.
    pub margin: f32,
    /// The popup window takes the anchor's font and color.
    pub inherit_text: bool,
    pub build: Build,
    /// Told where an in-window popup went, for hit testing.
    pub placed: fn(WinKey, u32, Bounds<Pixels>),
    /// Closes it as a press outside it does, in the window that opened it.
    pub dismiss: fn(&mut Window, &mut App),
}

impl Popup {
    pub fn into_element(self) -> AnyElement {
        if native() {
            let child = (self.build)();
            return Native { popup: self, child }.into_any_element();
        }
        gpui::deferred(Placed {
            anchor: self.anchor,
            fallback: self.fallback,
            fixed: None,
            placement: Placement::Below,
            gap: self.gap,
            child: (self.build)(),
            placed: self.placed,
            menu: None,
        })
        .priority(1)
        .with_stack_key(Rc::from([u128::MAX]))
        .into_any_element()
    }
}

static FAILED: AtomicBool = AtomicBool::new(false);

/// Whether popups open in windows of their own.
pub(crate) fn native() -> bool {
    !FAILED.load(Ordering::Relaxed)
}

/// Runs `f` in the window `k`: a popup window's handlers act on the window that opened it.
pub(crate) fn in_window(
    k: WinKey,
    window: &mut Window,
    cx: &mut App,
    f: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    if windows::key(window) == k {
        return f(window, cx);
    }
    let Some(h) = handle_of(k, cx) else { return };
    let mut f = Some(f);
    let _ = h.update(cx, |_, window, cx| {
        if let Some(f) = f.take() {
            f(window, cx)
        }
    });
    // The owner is mid-update when a devtools command drives the popup window from inside it.
    if let Some(f) = f {
        cx.defer(move |cx| {
            let _ = h.update(cx, |_, window, cx| f(window, cx));
        });
    }
}

pub(super) fn handle_of(k: WinKey, cx: &App) -> Option<gpui::AnyWindowHandle> {
    HOSTS
        .with(|h| {
            h.borrow()
                .iter()
                .find(|h| h.owner == k)
                .map(|h| h.owner_handle)
        })
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find(|h| h.window_id().as_u64() == k)
        })
}

struct Placed {
    menu: Option<(u32, bool)>,
    anchor: u32,
    fallback: Rect,
    fixed: Option<Rect>,
    placement: Placement,
    gap: f32,
    child: AnyElement,
    placed: fn(WinKey, u32, Bounds<Pixels>),
}

/// How far `rect` moves up so a submenu's first row lines up with its trigger: the menu's top
/// padding and border.
fn beside_rect(placement: Placement, root: u32, (x, y, w, h): Rect) -> Rect {
    if placement != Placement::Beside {
        return (x, y, w, h);
    }
    let px = |l: gpui::AbsoluteLength| match l {
        gpui::AbsoluteLength::Pixels(p) => f32::from(p),
        gpui::AbsoluteLength::Rems(r) => r.0 * sk_css::rem_basis(),
    };
    let inset = crate::tree::read_tree(|t| {
        let base = &t.node(root)?.computed.as_ref()?.base;
        let padding = match base.padding.top {
            Some(gpui::DefiniteLength::Absolute(l)) => px(l),
            _ => 0.,
        };
        Some(padding + base.border_widths.top.map_or(0., px))
    })
    .unwrap_or(0.);
    (x, y - inset, w, h)
}

fn anchor_box(k: WinKey, anchor: u32, fallback: Rect) -> Rect {
    windows::peek(k, |w| {
        w.bounds
            .get(&anchor)
            .or_else(|| w.prev_bounds.get(&anchor))
            .copied()
    })
    .flatten()
    .unwrap_or(fallback)
}

impl gpui::Element for Placed {
    type RequestLayoutState = gpui::LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, gpui::LayoutId) {
        let child = self.child.request_layout(window, cx);
        let style = gpui::Style {
            position: gpui::Position::Absolute,
            display: gpui::Display::Flex,
            ..Default::default()
        };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut gpui::LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let laid = window.layout_bounds(*child);
        let k = windows::key(window);
        let rect = self
            .fixed
            .unwrap_or_else(|| anchor_box(k, self.anchor, self.fallback));
        let rect = match self.menu {
            Some((root, _)) => beside_rect(self.placement, root, rect),
            None => rect,
        };
        let viewport = window.viewport_size();
        let (x, y) = place_at(
            self.placement,
            rect,
            self.gap,
            (f32::from(laid.size.width), f32::from(laid.size.height)),
            (
                0.0,
                0.0,
                f32::from(viewport.width),
                f32::from(viewport.height),
            ),
        );
        let origin = gpui::point(px(x), px(y));
        let offset = origin - laid.origin;
        let offset = gpui::point(offset.x.round(), offset.y.round());
        window.with_element_offset(offset, |window| self.child.prepaint(window, cx));
        if let Some((root, _)) = self.menu {
            let painted = Bounds::new(laid.origin + offset, laid.size);
            crate::context_menu::placed(root, crate::render::rect_of(painted), cx);
        }
        (self.placed)(
            k,
            self.anchor,
            Bounds {
                origin,
                size: laid.size,
            },
        );
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _child: &mut gpui::LayoutId,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        if let Some((root, true)) = self.menu {
            crate::context_menu::aim::paint(root, window);
        }
    }
}

/// Lays the popup out in the opening window for its size, paints nothing there, and shows the
/// popup window.
struct Native {
    popup: Popup,
    child: AnyElement,
}

impl gpui::Element for Native {
    type RequestLayoutState = gpui::LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, gpui::LayoutId) {
        let child = self.child.request_layout(window, cx);
        let style = gpui::Style {
            position: gpui::Position::Absolute,
            display: gpui::Display::Flex,
            ..Default::default()
        };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut gpui::LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let size = window.layout_bounds(*child).size;
        let p = &self.popup;
        let text = p.inherit_text.then(|| {
            let ts = window.text_style();
            Text {
                font: ts.font(),
                size: ts.font_size.to_pixels(window.rem_size()),
                color: ts.color,
            }
        });
        let rect = anchor_box(windows::key(window), p.anchor, p.fallback);
        show(
            Request {
                slot: p.slot,
                anchor: p.anchor,
                rect,
                gap: p.gap,
                size: (f32::from(size.width), f32::from(size.height)),
                content: Content {
                    body: Body::Widget(p.build.clone()),
                    margin: p.margin,
                    text,
                    dismiss: p.dismiss,
                },
            },
            window,
            cx,
        );
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _child: &mut gpui::LayoutId,
        _prepaint: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

pub(crate) fn menu_element(
    menu: crate::context_menu::Menu,
    fixed: Option<Rect>,
    mut child: AnyElement,
) -> AnyElement {
    if !native() {
        return gpui::deferred(Placed {
            menu: Some((menu.root, menu.debug_aim)),
            anchor: menu.anchor,
            fallback: (0., 0., 0., 0.),
            fixed,
            placement: menu.placement,
            gap: 0.,
            child,
            placed: |_, _, _| {},
        })
        .priority(1)
        .with_stack_key(Rc::from([u128::MAX]))
        .into_any_element();
    }
    gpui::canvas(
        move |_, window, cx| {
            let max = gpui::AvailableSpace::MaxContent;
            let size = child.layout_as_root(gpui::size(max, max), window, cx);
            let rect = fixed
                .unwrap_or_else(|| anchor_box(windows::key(window), menu.anchor, (0., 0., 0., 0.)));
            let rect = beside_rect(menu.placement, menu.root, rect);
            show(
                Request {
                    slot: Slot::Menu(menu.root),
                    anchor: menu.anchor,
                    rect,
                    gap: 0.,
                    size: (
                        f32::from(size.width).max(1.),
                        f32::from(size.height).max(1.),
                    ),
                    content: Content {
                        body: Body::Menu(menu),
                        margin: 0.,
                        text: None,
                        dismiss: |_, _| {},
                    },
                },
                window,
                cx,
            );
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
    .into_any_element()
}

/// Where a popup of `size` goes against the box `rect`, inside `area`.
fn place_at(
    placement: Placement,
    rect: Rect,
    gap: f32,
    (pw, ph): (f32, f32),
    area: Rect,
) -> (f32, f32) {
    let (x, y, w, h) = rect;
    if placement == Placement::Beside {
        let (ax, ay, aw, ah) = area;
        let right = x + w + gap;
        let left = x - gap - pw;
        let x = if right + pw <= ax + aw || (left < ax && ax + aw - right >= x - gap - ax) {
            right
        } else {
            left
        };
        return (x.min(ax + aw - pw).max(ax), y.min(ay + ah - ph).max(ay));
    }
    let (ax, ay, aw, ah) = area;
    let below = ay + ah - (y + h + gap);
    let above = y - gap - ay;
    let top = if ph <= below || below >= above {
        y + h + gap
    } else {
        (y - gap - ph).max(ay)
    };
    let left = if placement == Placement::Point && x + pw > ax + aw {
        x - pw
    } else {
        x
    };
    (left.min(ax + aw - pw).max(ax), top)
}

/// The most room a popup under or above `rect` has in `work`, both in screen units, over `scale`.
fn room_in(rect: Rect, work: Rect, gap: f32, scale: f32) -> f32 {
    let (_, ay, _, ah) = work;
    let below = ay + ah - (rect.1 + rect.3 + gap * scale);
    let above = rect.1 - gap * scale - ay;
    below.max(above).max(0.0) / scale
}

/// A popup window's frame in screen units, around content placed against `rect` inside `work`,
/// with `margin` room on each side: `[x, y, w, h]`, whole device pixels.
fn frame_in(
    placement: Placement,
    rect: Rect,
    work: Rect,
    gap: f32,
    (w, h): (f32, f32),
    margin: f32,
    scale: f32,
) -> [i32; 4] {
    let s = scale;
    let m = margin * s;
    let (w, h) = ((w * s).min(work.2), (h * s).min(work.3));
    let (x, y) = place_at(placement, rect, gap * s, (w, h), work);
    [
        (x - m).round() as i32,
        (y.min(work.1 + work.3 - h).max(work.1) - m).round() as i32,
        (w + 2.0 * m).ceil() as i32,
        (h + 2.0 * m).ceil() as i32,
    ]
}

/// Maps a placed frame back to the opener's CSS pixels using the anchor common to both spaces.
fn in_owner(frame: [i32; 4], local: Rect, screen: Rect, owner_scale: f32) -> Rect {
    let [x, y, w, h] = frame.map(|v| v as f32);
    (
        local.0 + (x - screen.0) / owner_scale,
        local.1 + (y - screen.1) / owner_scale,
        w / owner_scale,
        h / owner_scale,
    )
}

struct Text {
    font: gpui::Font,
    size: Pixels,
    color: Hsla,
}

enum Body {
    Widget(Build),
    Menu(crate::context_menu::Menu),
}

struct Content {
    body: Body,
    margin: f32,
    text: Option<Text>,
    dismiss: fn(&mut Window, &mut App),
}

struct Request {
    slot: Slot,
    anchor: u32,
    rect: Rect,
    gap: f32,
    size: (f32, f32),
    content: Content,
}

struct Host {
    slot: Slot,
    anchor: u32,
    handle: gpui::AnyWindowHandle,
    owner: WinKey,
    owner_handle: gpui::AnyWindowHandle,
    owner_bounds: Bounds<Pixels>,
    content: Rc<RefCell<Content>>,
    native: plat::Native,
    place: plat::Place,
}

thread_local! {
    static HOSTS: RefCell<Vec<Host>> = const { RefCell::new(Vec::new()) };
    /// What each slot's last frame showed; a close after it cancels the window it asked for.
    static WANTED: RefCell<Vec<(Slot, u32)>> = const { RefCell::new(Vec::new()) };
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

fn wanted(slot: Slot) -> Option<u32> {
    WANTED.with(|w| w.borrow().iter().find(|(s, _)| *s == slot).map(|(_, a)| *a))
}

fn set_wanted(slot: Slot, anchor: Option<u32>) {
    WANTED.with(|w| {
        let mut w = w.borrow_mut();
        w.retain(|(s, _)| *s != slot);
        w.extend(anchor.map(|a| (slot, a)));
    });
}

struct Root {
    content: Rc<RefCell<Content>>,
    owner: WinKey,
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let c = self.content.borrow();
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .items_start()
            .p(px(c.margin));
        if let Some(t) = &c.text {
            root = root
                .font(t.font.clone())
                .text_size(t.size)
                .text_color(t.color);
        }
        if c.margin > 0.0 {
            // A press on the shadow's room is one outside the popup (Windows and X11 pass it on).
            let (owner, dismiss, m) = (self.owner, c.dismiss, px(c.margin));
            root = root.on_any_mouse_down(move |e, window, cx| {
                let v = window.viewport_size();
                let p = e.position;
                if p.x < m || p.y < m || p.x >= v.width - m || p.y >= v.height - m {
                    in_window(owner, window, cx, dismiss);
                }
            });
        }
        match &c.body {
            Body::Widget(build) => root.child(build()),
            Body::Menu(menu) => {
                windows::set_painted_root(windows::key(window), menu.root);
                if let Some(bounds) = window.popup_bounds_in_parent() {
                    crate::context_menu::placed(menu.root, crate::render::rect_of(bounds), cx);
                }
                let fill = (menu.prepare_window)(window, cx);
                if let Some(fill) =
                    fill.filter(|_| menu.backdrop == gpui::WindowBackgroundAppearance::Opaque)
                {
                    root = root.bg(fill);
                }
                crate::window_content_tick(window, cx);
                let id = menu.root;
                root.child(crate::render::build_subtree(id, window, cx))
                    .when(menu.debug_aim, |root| {
                        root.child(
                            gpui::canvas(
                                |_, _, _| (),
                                move |_, _, window, _| {
                                    crate::context_menu::aim::paint(id, window);
                                },
                            )
                            .absolute()
                            .inset_0(),
                        )
                    })
            }
        }
    }
}

/// The popup windows open now: `(window, the window that opened it, margin)`.
pub fn popup_windows() -> Vec<(WinKey, WinKey, f32)> {
    HOSTS.with(|h| {
        h.borrow()
            .iter()
            .map(|h| {
                (
                    h.handle.window_id().as_u64(),
                    h.owner,
                    h.content.borrow().margin,
                )
            })
            .collect()
    })
}

/// Closes `slot`'s popup window if it hangs from `anchor`.
pub(crate) fn close(slot: Slot, anchor: u32, cx: &mut App) {
    if wanted(slot) == Some(anchor) {
        set_wanted(slot, None);
    }
    let gone = HOSTS.with(|h| {
        let mut h = h.borrow_mut();
        let at = h
            .iter()
            .position(|h| h.slot == slot && h.anchor == anchor)?;
        Some(h.remove(at).handle)
    });
    if let Some(handle) = gone {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        });
    }
}

/// The window that opened `slot`'s popup window moved or was resized since it did.
pub(crate) fn owner_moved(slot: Slot, window: &Window) -> bool {
    let k = windows::key(window);
    let now = window.bounds();
    HOSTS.with(|h| {
        h.borrow()
            .iter()
            .any(|h| h.slot == slot && h.owner == k && h.owner_bounds != now)
    })
}

/// The most room a popup window under or above `rect` has on its monitor, in `window`'s px.
pub(crate) fn room(window: &Window, rect: Rect, gap: f32) -> Option<f32> {
    plat::room(window, rect, gap)
}

fn show(req: Request, window: &mut Window, cx: &mut App) {
    set_wanted(req.slot, Some(req.anchor));
    let k = windows::key(window);
    let placement = match &req.content.body {
        Body::Menu(menu) => menu.placement,
        Body::Widget(_) => Placement::Below,
    };
    let (place, in_owner) = match plat::locate(
        window,
        req.rect,
        req.gap,
        req.size,
        req.content.margin,
        placement,
    ) {
        Ok(place) => place,
        Err(why) => return give_up(why, k, cx),
    };
    if let (Body::Menu(menu), Some(bounds)) = (&req.content.body, in_owner) {
        crate::context_menu::placed(menu.root, bounds, cx);
    }
    let owner_bounds = window.bounds();
    let owner_handle = window.window_handle();
    cx.defer(move |cx| apply(req, k, owner_handle, owner_bounds, place, cx));
}

/// Falls back to in-window popups for the rest of the run.
fn give_up(why: &str, owner: WinKey, cx: &mut App) {
    if FAILED.swap(true, Ordering::Relaxed) {
        return;
    }
    log::warn!("popups are drawn inside the window: {why}");
    if let Some(h) = handle_of(owner, cx) {
        let _ = h.update(cx, |_, window, cx| {
            let root = windows::root_of(windows::key(window));
            crate::tree::with_tree(|t| t.touch(root));
            crate::render::request_repaint(window, cx);
        });
    }
}

fn apply(
    req: Request,
    owner: WinKey,
    owner_handle: gpui::AnyWindowHandle,
    owner_bounds: Bounds<Pixels>,
    place: plat::Place,
    cx: &mut App,
) {
    let Request {
        slot,
        anchor,
        content,
        ..
    } = req;
    if wanted(slot) != Some(anchor) {
        return;
    }
    let mut content = Some(content);
    let current = HOSTS.with(|h| {
        let mut h = h.borrow_mut();
        let at = h.iter().position(|h| h.slot == slot)?;
        if h[at].owner != owner {
            return Some(Err(h.remove(at).handle));
        }
        if !plat::alive(&h[at].native) {
            h.remove(at);
            return None;
        }
        let host = &mut h[at];
        host.anchor = anchor;
        *host.content.borrow_mut() = content.take()?;
        let from = std::mem::replace(&mut host.place, place.clone());
        Some(Ok((
            host.handle,
            host.native.clone(),
            from,
            host.content.clone(),
        )))
    });
    let content = match current {
        Some(Ok((handle, native, from, kept))) => {
            if from == place || plat::update(&native, &from, &place, handle, cx) {
                // Fails only while a devtools command drives it, which refreshes it after.
                let _ = handle.update(cx, |_, window, _| window.refresh());
                return;
            }
            HOSTS.with(|h| h.borrow_mut().retain(|h| h.handle != handle));
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            kept
        }
        Some(Err(stale)) => {
            let _ = stale.update(cx, |_, window, _| window.remove_window());
            return;
        }
        None => match content {
            Some(c) => Rc::new(RefCell::new(c)),
            None => return,
        },
    };
    let (bounds, kind, display_id) = plat::options(&place, owner_handle, cx);
    let options = gpui::WindowOptions {
        window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: false,
        kind,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        // Never active, and it must keep up with a drag.
        inactive_frame_interval: None,
        low_power_frame_interval: None,
        display_id,
        window_background: match &content.borrow().body {
            Body::Widget(_) => gpui::WindowBackgroundAppearance::Transparent,
            Body::Menu(menu) => menu.backdrop,
        },
        ..Default::default()
    };
    let root = content.clone();
    let opened = cx.open_window(options, move |window, cx| {
        let window_id = windows::key(window);
        cx.new(|cx| {
            if matches!(slot, Slot::Menu(_)) {
                cx.on_release(move |_, _| crate::forget_window(window_id))
                    .detach();
            }
            Root {
                content: root,
                owner,
            }
        })
    });
    let handle: gpui::AnyWindowHandle = match opened {
        Ok(h) => h.into(),
        Err(e) => return give_up(&format!("opening its window failed: {e}"), owner, cx),
    };
    let attached = handle
        .update(cx, |_, window, _| {
            plat::attach(window, &place, matches!(slot, Slot::Menu(_)))
        })
        .unwrap_or(Err("the popup window closed as it opened"));
    let native = match attached {
        Ok(native) => native,
        Err(why) => {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            return give_up(why, owner, cx);
        }
    };
    watch_closes(cx);
    HOSTS.with(|h| {
        h.borrow_mut().push(Host {
            slot,
            anchor,
            handle,
            owner,
            owner_handle,
            owner_bounds,
            content,
            native,
            place,
        })
    });
}

/// Popup windows go with the window that opened them; one closed from outside (a Wayland
/// compositor dismissing it) closes its popup as a press outside it would.
fn watch_closes(cx: &mut App) {
    if WATCHING.with(|w| w.replace(true)) {
        return;
    }
    cx.on_window_closed(|cx, id| {
        let id = id.as_u64();
        let mut orphan_menus = Vec::new();
        let mut orphans = Vec::new();
        let mut lost = None;
        HOSTS.with(|h| {
            h.borrow_mut().retain(|h| {
                if h.owner == id {
                    if let Slot::Menu(root) = h.slot {
                        orphan_menus.push(root);
                    }
                    orphans.push(h.handle);
                    false
                } else if h.handle.window_id().as_u64() == id {
                    lost = Some((h.slot, h.anchor, h.owner, h.content.borrow().dismiss));
                    false
                } else {
                    true
                }
            })
        });
        for root in orphan_menus {
            cx.defer(move |cx| crate::context_menu::dismiss(root, cx));
        }
        for handle in orphans {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
        if let Some((slot, anchor, owner, dismiss)) = lost {
            if wanted(slot) == Some(anchor) {
                set_wanted(slot, None);
                if let Slot::Menu(root) = slot {
                    cx.defer(move |cx| crate::context_menu::dismiss(root, cx));
                    return;
                }
                if let Some(h) = handle_of(owner, cx) {
                    cx.defer(move |cx| {
                        let _ = h.update(cx, |_, window, cx| dismiss(window, cx));
                    });
                }
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(rect: Rect, gap: f32, size: (f32, f32), area: Rect) -> (f32, f32) {
        place_at(Placement::Below, rect, gap, size, area)
    }

    #[test]
    fn the_popup_goes_under_the_box_and_above_it_when_only_that_fits() {
        let vp = (0., 0., 800., 600.);
        assert_eq!(
            place((10., 100., 200., 30.), 0., (200., 300.), vp),
            (10., 130.)
        );
        assert_eq!(
            place((10., 500., 200., 30.), 0., (200., 300.), vp),
            (10., 200.)
        );
        assert_eq!(
            place((10., 100., 200., 30.), 0., (200., 550.), vp),
            (10., 130.)
        );
        assert_eq!(
            place((700., 100., 80., 30.), 0., (200., 100.), vp),
            (600., 130.)
        );
        assert_eq!(
            place((10., 500., 50., 20.), 4., (200., 300.), vp),
            (10., 196.),
            "the gap is kept above too"
        );
    }

    #[test]
    fn menus_flip_at_the_pointer_and_beside_the_submenu_on_a_scaled_monitor() {
        let work = (-1440., 60., 1440., 900.);
        assert_eq!(
            frame_in(
                Placement::Point,
                (-12., 942., 0., 0.),
                work,
                0.,
                (180., 120.),
                0.,
                1.5
            ),
            [-282, 762, 270, 180]
        );
        assert_eq!(
            frame_in(
                Placement::Beside,
                (-210., 900., 192., 42.),
                work,
                0.,
                (180., 120.),
                0.,
                1.5
            ),
            [-480, 780, 270, 180]
        );
        assert_eq!(
            frame_in(
                Placement::Beside,
                (-1420., 100., 180., 30.),
                work,
                0.,
                (180., 120.),
                0.,
                1.5
            ),
            [-1240, 100, 270, 180]
        );
        assert_eq!(
            frame_in(
                Placement::Point,
                (-6., 900., 0., 0.),
                work,
                0.,
                (1100., 1600.),
                0.,
                1.5
            ),
            [-1440, 60, 1440, 900]
        );
    }

    #[test]
    fn on_a_monitor_the_popup_stays_in_its_work_area() {
        let work = (1920., 0., 1280., 984.);
        assert_eq!(
            place((3100., 900., 60., 20.), 4., (240., 400.), work),
            (2960., 496.),
            "above, and left of the right edge"
        );
        assert_eq!(
            place((1900., 100., 60., 20.), 0., (240., 400.), work),
            (1920., 120.),
            "not left of the monitor"
        );
    }
}
