//! `<SkWindow>` on macOS and Linux: what Windows does through the HWND, through gpui's platform
//! window. Monitor indices and the cover window's warm size stay Windows-only.

use gpui::{point, px, size, AnyWindowHandle, App, Window, WindowHandle, WindowId};

use super::PopoutShell;

/// The live props that changed; `None` = unchanged.
pub(super) struct Live {
    pub always_on_top: bool,
    pub click_through: Option<bool>,
    pub protection: Option<bool>,
    /// `[x, y, width, height]` of the client area in CSS px; `None` fields are kept.
    pub bounds: Option<[Option<f32>; 4]>,
}

pub(super) fn apply_live(handle: WindowHandle<PopoutShell>, live: Live, cx: &mut App) {
    let _ = AnyWindowHandle::from(handle).update(cx, |_, window, _| {
        window.vn_set_always_on_top(live.always_on_top);
        if let Some(on) = live.click_through {
            window.vn_set_ignore_mouse(on);
        }
        if let Some(on) = live.protection {
            window.vn_set_content_protection(on);
        }
        if let Some(bounds) = live.bounds {
            set_client_bounds(window, bounds);
        }
    });
}

fn set_client_bounds(window: &mut Window, [x, y, w, h]: [Option<f32>; 4]) {
    let frame = window.bounds();
    let client = window.viewport_size();
    // macOS with an OS title bar: the frame is taller than the client area by it.
    let title_bar = (frame.size.height - client.height).max(px(0.0));
    if w.is_some() || h.is_some() {
        window.resize(size(
            w.map_or(client.width, px),
            h.map_or(client.height, px),
        ));
    }
    if x.is_some() || y.is_some() {
        let origin = point(
            x.map_or(frame.origin.x, px),
            y.map_or(frame.origin.y, |y| px(y) - title_bar),
        );
        window.vn_set_frame(Some(origin), None);
    }
}

/// A hidden or hidden-first pop-out's show; `cover` fills the display it is on. `after` runs once
/// the window is on screen.
pub(super) fn show(
    id: WindowId,
    handle: WindowHandle<PopoutShell>,
    cover: bool,
    activate: bool,
    on_top: bool,
    after: impl FnOnce() + 'static,
    cx: &mut App,
) {
    let _ = AnyWindowHandle::from(handle).update(cx, |_, window, cx| {
        if cover {
            if let Some(display) = window.display(cx) {
                let b = display.bounds();
                window.vn_set_frame(Some(b.origin), Some(b.size));
            }
        }
        window.refresh();
    });
    // Outside this update: the platform's show may draw, and the draw takes the app.
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            let _ = AnyWindowHandle::from(handle).update(cx, |_, window, _| {
                let after = move |shown: bool| {
                    if shown {
                        after();
                    }
                };
                if !window.show_window(activate, || true, after) && activate {
                    window.activate_window();
                }
                if on_top {
                    window.vn_set_always_on_top(true);
                }
            });
            super::shown(id, cx);
        });
    })
    .detach();
}

/// Where a window cannot hide (Wayland) it is minimized instead. False once the window is gone.
pub(super) fn hide(id: WindowId, cx: &mut App) -> bool {
    let Some(handle) =
        super::POPOUTS.with(|p| p.borrow().iter().find(|w| w.id == id).map(|w| w.handle))
    else {
        return false;
    };
    AnyWindowHandle::from(handle)
        .update(cx, |_, window, _| {
            if !window.vn_hide() {
                window.minimize_window();
            }
        })
        .is_ok()
}
