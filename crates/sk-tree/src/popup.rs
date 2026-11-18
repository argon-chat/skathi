//! Popups (the color picker, a select's list): on Windows a borderless window of their own, placed
//! under the anchor's box on its monitor's work area, so they may extend past the app's window;
//! elsewhere drawn over the window's content, kept inside it. Under its anchor, or above when it
//! does not fit and fits better there, kept inside sideways (gpui's `anchored` flips around one
//! point, a popup around a box).
#![cfg_attr(not(windows), allow(dead_code))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::prelude::*;
use gpui::{div, px, AnyElement, App, Bounds, Hsla, Pixels, Window};

use crate::windows::{self, Rect, WinKey};

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
    cfg!(windows) && !FAILED.load(Ordering::Relaxed)
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

struct Text {
    font: gpui::Font,
    size: Pixels,
    color: Hsla,
}

struct Content {
    build: Build,
    margin: f32,
    text: Option<Text>,
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
    #[cfg(windows)]
    hwnd: win::HWND,
    #[cfg(windows)]
    spot: [i32; 4],
}

thread_local! {
    static HOSTS: RefCell<Vec<Host>> = const { RefCell::new(Vec::new()) };
    /// What each slot's last frame showed; a close after it cancels the window it asked for.
    static WANTED: RefCell<Vec<(Slot, u32)>> = const { RefCell::new(Vec::new()) };
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
    #[cfg(windows)]
    {
        let owner = win::hwnd(window)?;
        let scale = window.scale_factor();
        let (r, m) = win::anchor(owner, scale, rect)?;
        let (_, ay, _, ah) = m.work;
        let below = ay + ah - (r.1 + r.3 + gap * m.scale);
        let above = r.1 - gap * m.scale - ay;
        Some(below.max(above).max(0.0) / m.scale)
    }
    #[cfg(not(windows))]
    {
        let _ = (window, rect, gap);
        None
    }
}

fn show(req: Request, window: &mut Window, cx: &mut App) {
    set_wanted(req.slot, Some(req.anchor));
    #[cfg(windows)]
    {
        let k = windows::key(window);
        let Some(owner) = win::hwnd(window) else {
            return give_up("the window has no HWND", k, cx);
        };
        let Some((anchor, monitor)) = win::anchor(owner, window.scale_factor(), req.rect) else {
            return give_up("no monitor under the anchor", k, cx);
        };
        let s = monitor.scale;
        let m = req.content.margin * s;
        let (w, h) = (req.size.0 * s, req.size.1 * s);
        let (x, y) = place(anchor, req.gap * s, (w, h), monitor.work);
        let spot = [
            (x - m).round() as i32,
            (y - m).round() as i32,
            (w + 2.0 * m).ceil() as i32,
            (h + 2.0 * m).ceil() as i32,
        ];
        let inset = m.round() as i32;
        let owner_bounds = window.bounds();
        let owner_handle = window.window_handle();
        let raw_owner = owner.0 as isize;
        cx.defer(move |cx| {
            let owner = win::HWND(raw_owner as *mut core::ffi::c_void);
            apply(
                req,
                k,
                owner_handle,
                owner,
                owner_bounds,
                spot,
                inset,
                monitor,
                cx,
            )
        });
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        give_up(
            "no popup windows on this platform",
            windows::key(window),
            cx,
        );
    }
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

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn apply(
    req: Request,
    owner: WinKey,
    owner_handle: gpui::AnyWindowHandle,
    owner_hwnd: win::HWND,
    owner_bounds: Bounds<Pixels>,
    spot: [i32; 4],
    inset: i32,
    monitor: win::Monitor,
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
        if !win::alive(h[at].hwnd) {
            h.remove(at);
            return None;
        }
        let host = &mut h[at];
        host.anchor = anchor;
        *host.content.borrow_mut() = content.take()?;
        if host.spot != spot {
            host.spot = spot;
            win::place(host.hwnd, spot, inset, None);
        }
        Some(Ok(host.handle))
    });
    let content = match current {
        Some(Ok(handle)) => {
            // Fails only while a devtools command drives it, which refreshes it after.
            let _ = handle.update(cx, |_, window, _| window.refresh());
            return;
        }
        Some(Err(stale)) => {
            let _ = stale.update(cx, |_, window, _| window.remove_window());
            return;
        }
        None => match content {
            Some(c) => c,
            None => return,
        },
    };
    let content = Rc::new(RefCell::new(content));
    let s = monitor.scale;
    let bounds = Bounds {
        origin: gpui::point(px(spot[0] as f32 / s), px(spot[1] as f32 / s)),
        size: gpui::size(px(spot[2] as f32 / s), px(spot[3] as f32 / s)),
    };
    let display_id = cx
        .displays()
        .into_iter()
        .find(|d| {
            let o = d.bounds().origin;
            (f32::from(o.x) - monitor.bounds.0 / s).abs() < 1.0
                && (f32::from(o.y) - monitor.bounds.1 / s).abs() < 1.0
        })
        .map(|d| d.id());
    let options = gpui::WindowOptions {
        window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: false,
        kind: gpui::WindowKind::PopUp,
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
    let opened = cx.open_window(options, move |_, cx| cx.new(|_| Root { content: root }));
    let handle: gpui::AnyWindowHandle = match opened {
        Ok(h) => h.into(),
        Err(e) => return give_up(&format!("opening its window failed: {e}"), owner, cx),
    };
    let shown = handle
        .update(cx, |_, window, _| {
            let hwnd = win::hwnd(window)?;
            win::configure(hwnd, owner_hwnd);
            win::place(hwnd, spot, inset, Some(owner_hwnd));
            Some(hwnd)
        })
        .ok()
        .flatten();
    let Some(hwnd) = shown else {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        return give_up("the popup window has no HWND", owner, cx);
    };
    HOSTS.with(|h| {
        h.borrow_mut().push(Host {
            slot,
            anchor,
            handle,
            owner,
            owner_handle,
            owner_bounds,
            content,
            hwnd,
            spot,
        })
    });
}

#[cfg(windows)]
mod win {
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub(super) use windows::Win32::Foundation::HWND;
    use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GetWindow, GetWindowLongPtrW, GetWindowRect, IsWindow,
        SetWindowLongPtrW, SetWindowPos, GWLP_HWNDPARENT, GWLP_WNDPROC, GWL_EXSTYLE, GWL_STYLE,
        GW_HWNDPREV, HTTRANSPARENT, HWND_NOTOPMOST, MA_NOACTIVATE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SWP_SHOWWINDOW, WM_MOUSEACTIVATE,
        WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WNDPROC, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
        WS_EX_TOPMOST, WS_POPUP,
    };

    use crate::windows::Rect;

    pub(super) struct Monitor {
        /// The work area, device px.
        pub work: Rect,
        pub bounds: Rect,
        pub scale: f32,
    }

    pub(super) fn alive(hwnd: HWND) -> bool {
        // SAFETY: only reads whether the handle names a window.
        unsafe { IsWindow(Some(hwnd)) }.as_bool()
    }

    pub(super) fn hwnd(window: &gpui::Window) -> Option<HWND> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
            _ => None,
        }
    }

    /// `r` (client px of `owner`) on the screen in device px, and the monitor it is on.
    pub(super) fn anchor(owner: HWND, scale: f32, r: Rect) -> Option<(Rect, Monitor)> {
        let mut p = POINT {
            x: (r.0 * scale).round() as i32,
            y: (r.1 * scale).round() as i32,
        };
        unsafe { ClientToScreen(owner, &mut p) }.ok().ok()?;
        let (w, h) = (r.2 * scale, r.3 * scale);
        let rect = RECT {
            left: p.x,
            top: p.y,
            right: p.x + w.round() as i32,
            bottom: p.y + h.round() as i32,
        };
        let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetMonitorInfoW(monitor, &mut info) }.ok().ok()?;
        let (mut dx, mut dy) = (96u32, 96u32);
        let dpi = match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) } {
            Ok(()) => dx,
            Err(_) => 96,
        };
        let of = |r: RECT| {
            (
                r.left as f32,
                r.top as f32,
                (r.right - r.left) as f32,
                (r.bottom - r.top) as f32,
            )
        };
        Some((
            (p.x as f32, p.y as f32, w, h),
            Monitor {
                work: of(info.rcWork),
                bounds: of(info.rcMonitor),
                scale: dpi as f32 / 96.0,
            },
        ))
    }

    thread_local! {
        /// A popup window's own procedure, and how far in its content starts, device px.
        static PROCS: RefCell<HashMap<isize, (isize, i32)>> = RefCell::new(HashMap::new());
    }

    /// Owned by `owner` (above it, gone with it) and never activated, so `owner` keeps the
    /// keyboard and looks active; square, borderless corners: the content draws its own.
    pub(super) fn configure(popup: HWND, owner: HWND) {
        // SAFETY: both are live windows of this thread; the replaced procedure is kept and called.
        unsafe {
            SetWindowLongPtrW(popup, GWL_STYLE, (WS_POPUP | WS_CLIPSIBLINGS).0 as isize);
            let ex = GetWindowLongPtrW(popup, GWL_EXSTYLE);
            SetWindowLongPtrW(popup, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
            SetWindowLongPtrW(popup, GWLP_HWNDPARENT, owner.0 as isize);
            let previous = SetWindowLongPtrW(popup, GWLP_WNDPROC, procedure as *const () as isize);
            PROCS.with(|p| p.borrow_mut().insert(popup.0 as isize, (previous, 0)));
            let corners = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(
                popup,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corners).cast(),
                std::mem::size_of_val(&corners) as u32,
            );
            let border = DWMWA_COLOR_NONE;
            let _ = DwmSetWindowAttribute(
                popup,
                DWMWA_BORDER_COLOR,
                (&raw const border).cast(),
                std::mem::size_of_val(&border) as u32,
            );
        }
    }

    /// Moves `popup`; with `owner`, also shows it right above `owner` in the z-order.
    pub(super) fn place(popup: HWND, [x, y, w, h]: [i32; 4], inset: i32, owner: Option<HWND>) {
        PROCS.with(|p| {
            if let Some(e) = p.borrow_mut().get_mut(&(popup.0 as isize)) {
                e.1 = inset;
            }
        });
        let (after, flags) = match owner {
            Some(owner) => {
                // Behind the window over the owner, which also drops gpui's topmost; at the top of
                // the ordinary windows when only topmost ones are over it.
                // SAFETY: reads the z-order.
                let over = unsafe { GetWindow(owner, GW_HWNDPREV) }.ok().filter(|&w| {
                    w != popup
                        && unsafe { GetWindowLongPtrW(w, GWL_EXSTYLE) } & WS_EX_TOPMOST.0 as isize
                            == 0
                });
                (
                    Some(over.unwrap_or(HWND_NOTOPMOST)),
                    SWP_SHOWWINDOW | SWP_FRAMECHANGED,
                )
            }
            None => (None, SWP_NOZORDER),
        };
        // SAFETY: a live window of this thread.
        let _ = unsafe {
            SetWindowPos(
                popup,
                after,
                x,
                y,
                w,
                h,
                flags | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
    }

    unsafe extern "system" fn procedure(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let Some((previous, inset)) = PROCS.with(|p| p.borrow().get(&(hwnd.0 as isize)).copied())
        else {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        };
        match msg {
            // gpui activates a window on any press.
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            // No frame: the whole window is client area.
            WM_NCCALCSIZE if wparam.0 != 0 => return LRESULT(0),
            // The shadow's room is not the popup: presses there reach the window under it.
            WM_NCHITTEST if inset > 0 => {
                let (x, y) = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
                let mut r = RECT::default();
                let outside = unsafe { GetWindowRect(hwnd, &mut r) }.is_ok()
                    && (x < r.left + inset
                        || x >= r.right - inset
                        || y < r.top + inset
                        || y >= r.bottom - inset);
                if outside {
                    return LRESULT(HTTRANSPARENT as isize);
                }
            }
            WM_NCDESTROY => {
                PROCS.with(|p| p.borrow_mut().remove(&(hwnd.0 as isize)));
            }
            _ => {}
        }
        // SAFETY: `previous` is the procedure `SetWindowLongPtrW` replaced.
        let previous: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
        unsafe { CallWindowProcW(previous, hwnd, msg, wparam, lparam) }
    }
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
