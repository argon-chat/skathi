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
            gap: self.gap,
            child: (self.build)(),
            placed: self.placed,
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

fn handle_of(k: WinKey, cx: &App) -> Option<gpui::AnyWindowHandle> {
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
    anchor: u32,
    fallback: Rect,
    gap: f32,
    child: AnyElement,
    placed: fn(WinKey, u32, Bounds<Pixels>),
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
        let rect = anchor_box(k, self.anchor, self.fallback);
        let viewport = window.viewport_size();
        let (x, y) = place(
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
        window.with_element_offset(gpui::point(offset.x.round(), offset.y.round()), |window| {
            self.child.prepaint(window, cx)
        });
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
                    build: p.build.clone(),
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

/// Where a popup of `size` goes against the box `rect`, inside `area`.
pub(crate) fn place(rect: Rect, gap: f32, (pw, ph): (f32, f32), area: Rect) -> (f32, f32) {
    let (x, y, _, h) = rect;
    let (ax, ay, aw, ah) = area;
    let below = ay + ah - (y + h + gap);
    let above = y - gap - ay;
    let top = if ph <= below || below >= above {
        y + h + gap
    } else {
        (y - gap - ph).max(ay)
    };
    let left = x.min(ax + aw - pw).max(ax);
    (left, top)
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
    rect: Rect,
    work: Rect,
    gap: f32,
    (w, h): (f32, f32),
    margin: f32,
    scale: f32,
) -> [i32; 4] {
    let s = scale;
    let m = margin * s;
    let (w, h) = (w * s, h * s);
    let (x, y) = place(rect, gap * s, (w, h), work);
    [
        (x - m).round() as i32,
        (y - m).round() as i32,
        (w + 2.0 * m).ceil() as i32,
        (h + 2.0 * m).ceil() as i32,
    ]
}

struct Text {
    font: gpui::Font,
    size: Pixels,
    color: Hsla,
}

struct Content {
    build: Build,
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
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
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
        root.child((c.build)())
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
    let place = match plat::locate(window, req.rect, req.gap, req.size, req.content.margin) {
        Ok(place) => place,
        Err(why) => return give_up(why, k, cx),
    };
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
        window_background: gpui::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let root = content.clone();
    let opened = cx.open_window(options, move |_, cx| {
        cx.new(|_| Root {
            content: root,
            owner,
        })
    });
    let handle: gpui::AnyWindowHandle = match opened {
        Ok(h) => h.into(),
        Err(e) => return give_up(&format!("opening its window failed: {e}"), owner, cx),
    };
    let attached = handle
        .update(cx, |_, window, _| plat::attach(window, &place))
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
        let mut orphans = Vec::new();
        let mut lost = None;
        HOSTS.with(|h| {
            h.borrow_mut().retain(|h| {
                if h.owner == id {
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
        for handle in orphans {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
        if let Some((slot, anchor, owner, dismiss)) = lost {
            if wanted(slot) == Some(anchor) {
                set_wanted(slot, None);
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
