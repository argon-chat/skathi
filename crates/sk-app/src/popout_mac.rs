//! AppKit helpers for a `<SkWindow>` on macOS: the child window behind `taskbar={false}` and the
//! screen rects behind `mapFromMain()`. They take gpui's `NSView` and look its window up on every
//! call, because a view can be moved into another window.

#[cfg(target_os = "macos")]
mod imp {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send, MainThreadMarker};
    use objc2_foundation::{NSPoint, NSRect};
    use raw_window_handle::RawWindowHandle;

    /// `[x, y, width, height]` in AppKit's global screen space: points, y up.
    pub type Rect = [f64; 4];

    const FULL_SCREEN: usize = 1 << 14;
    const ORDERED_ABOVE: isize = 1;

    pub fn view_of(handle: Option<RawWindowHandle>) -> usize {
        match handle {
            Some(RawWindowHandle::AppKit(h)) => h.ns_view.as_ptr() as usize,
            _ => 0,
        }
    }

    fn window_of(view: usize) -> Option<&'static AnyObject> {
        MainThreadMarker::new()?;
        if view == 0 {
            return None;
        }
        // SAFETY: gpui's live `NSView`, on the main thread; its window outlives this call.
        unsafe {
            let window: *mut AnyObject = msg_send![view as *mut AnyObject, window];
            window.as_ref()
        }
    }

    fn visible(window: &AnyObject) -> bool {
        // SAFETY: `NSWindow` getters on the main thread.
        unsafe { msg_send![window, isVisible] }
    }

    fn fullscreen(window: &AnyObject) -> bool {
        // SAFETY: as above.
        let mask: usize = unsafe { msg_send![window, styleMask] };
        mask & FULL_SCREEN != 0
    }

    fn parent(window: &AnyObject) -> *mut AnyObject {
        // SAFETY: as above.
        unsafe { msg_send![window, parentWindow] }
    }

    /// Makes `child`'s window a child of `parent`'s, ordered above it. False when that cannot be
    /// done now: either window is hidden, the parent is minimized, or the child is in fullscreen.
    pub fn attach(child: usize, parent_view: usize) -> bool {
        let (Some(c), Some(p)) = (window_of(child), window_of(parent_view)) else {
            return false;
        };
        if std::ptr::eq(parent(c), p) {
            return true;
        }
        // SAFETY: `NSWindow` getters on the main thread.
        let minimized: bool = unsafe { msg_send![p, isMiniaturized] };
        if !visible(c) || !visible(p) || minimized || fullscreen(c) {
            return false;
        }
        detach(child);
        // SAFETY: two live windows of this app, on the main thread.
        unsafe {
            let _: () = msg_send![p, addChildWindow: c, ordered: ORDERED_ABOVE];
        }
        true
    }

    pub fn detach(child: usize) {
        let Some(c) = window_of(child) else { return };
        let p = parent(c);
        if !p.is_null() {
            // SAFETY: `p` is `c`'s parent window, alive while it has the child.
            unsafe {
                let _: () = msg_send![p, removeChildWindow: c];
            }
        }
    }

    pub fn is_child_of(child: usize, parent_view: usize) -> bool {
        match (window_of(child), window_of(parent_view)) {
            (Some(c), Some(p)) => std::ptr::eq(parent(c), p),
            _ => false,
        }
    }

    pub fn is_visible(view: usize) -> bool {
        window_of(view).is_some_and(visible)
    }

    pub fn is_key(view: usize) -> bool {
        window_of(view).is_some_and(|w| unsafe { msg_send![w, isKeyWindow] })
    }

    /// The view's rect on screen.
    pub fn view_on_screen(view: usize) -> Option<Rect> {
        let window = window_of(view)?;
        // SAFETY: the view and its window, on the main thread.
        let r: NSRect = unsafe {
            let bounds: NSRect = msg_send![view as *mut AnyObject, bounds];
            let in_window: NSRect = msg_send![view as *mut AnyObject, convertRect: bounds, toView: std::ptr::null_mut::<AnyObject>()];
            msg_send![window, convertRectToScreen: in_window]
        };
        Some([r.origin.x, r.origin.y, r.size.width, r.size.height])
    }

    /// Where the view will be once its window fills the frame of the screen it is on, or of the
    /// screen `fallback`'s window is on.
    pub fn view_on_filled_screen(view: usize, fallback: usize) -> Option<Rect> {
        let window = window_of(view)?;
        let [vx, vy, vw, vh] = view_on_screen(view)?;
        // SAFETY: `NSWindow`/`NSScreen` getters on the main thread.
        unsafe {
            let frame: NSRect = msg_send![window, frame];
            let mut screen: *mut AnyObject = msg_send![window, screen];
            if screen.is_null() {
                screen = msg_send![window_of(fallback)?, screen];
            }
            let s: NSRect = msg_send![screen.as_ref()?, frame];
            let left = vx - frame.origin.x;
            let bottom = vy - frame.origin.y;
            Some([
                s.origin.x + left,
                s.origin.y + bottom,
                s.size.width - (frame.size.width - vw),
                s.size.height - (frame.size.height - vh),
            ])
        }
    }

    /// Queues a mouse move, or a move and a left click, at the view's `(x, y)` (points from its
    /// top-left) on the app's event queue, so it goes through `NSApplication` and `NSWindow` as
    /// real input does. The cursor does not move.
    pub fn post_mouse(view: usize, x: f64, y: f64, click: bool) -> bool {
        const MOVED: usize = 5;
        const DOWN: usize = 1;
        const UP: usize = 2;
        let Some(window) = window_of(view) else {
            return false;
        };
        let none = std::ptr::null_mut::<AnyObject>();
        // SAFETY: AppKit class methods and the live view/window, on the main thread.
        unsafe {
            let bounds: NSRect = msg_send![view as *mut AnyObject, bounds];
            let at = NSPoint::new(x, bounds.size.height - y);
            let location: NSPoint =
                msg_send![view as *mut AnyObject, convertPoint: at, toView: none];
            let number: isize = msg_send![window, windowNumber];
            let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
            let now: f64 = msg_send![info, systemUptime];
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let kinds: &[usize] = if click { &[MOVED, DOWN, UP] } else { &[MOVED] };
            for &kind in kinds {
                let event: *mut AnyObject = msg_send![
                    class!(NSEvent),
                    mouseEventWithType: kind,
                    location: location,
                    modifierFlags: 0usize,
                    timestamp: now,
                    windowNumber: number,
                    context: none,
                    eventNumber: 0isize,
                    clickCount: if kind == MOVED { 0isize } else { 1 },
                    pressure: if kind == DOWN { 1.0f32 } else { 0.0 }
                ];
                if event.is_null() {
                    return false;
                }
                let _: () = msg_send![app, postEvent: event, atStart: false];
            }
        }
        true
    }

    /// The window's frame as `[left, top, right, bottom]`, points, with y down from the top of
    /// the primary screen.
    pub fn frame_top_left(view: usize) -> Option<[f64; 4]> {
        let window = window_of(view)?;
        // SAFETY: `NSWindow`/`NSScreen` getters on the main thread.
        unsafe {
            let f: NSRect = msg_send![window, frame];
            let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
            let primary: *mut AnyObject = msg_send![screens.as_ref()?, firstObject];
            let p: NSRect = msg_send![primary.as_ref()?, frame];
            let top = p.size.height - (f.origin.y + f.size.height);
            Some([
                f.origin.x,
                top,
                f.origin.x + f.size.width,
                top + f.size.height,
            ])
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::*;

/// Maps a rect between two views (CSS px = points, y down from each view's top) through AppKit's
/// global screen space, where `from` and `to` are the views' rects on screen (y up).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn map_rect(rect: (f32, f32, f32, f32), from: [f64; 4], to: [f64; 4]) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = rect;
    let from_top = from[1] + from[3];
    let to_top = to[1] + to[3];
    (
        (from[0] + f64::from(x) - to[0]) as f32,
        (to_top - from_top + f64::from(y)) as f32,
        w,
        h,
    )
}

#[cfg(test)]
mod tests {
    use super::map_rect;

    #[test]
    fn a_rect_crosses_views_through_the_y_up_screen() {
        let main = [100.0, 200.0, 800.0, 600.0];
        assert_eq!(
            map_rect((10.0, 20.0, 30.0, 40.0), main, main),
            (10.0, 20.0, 30.0, 40.0)
        );
        let popout = [300.0, 500.0, 200.0, 100.0];
        assert_eq!(
            map_rect((210.0, 200.0, 30.0, 40.0), main, popout),
            (10.0, 0.0, 30.0, 40.0)
        );
        let left_display = [-1440.0, 300.0, 400.0, 300.0];
        assert_eq!(
            map_rect((0.0, 0.0, 50.0, 20.0), main, left_display),
            (1540.0, -200.0, 50.0, 20.0)
        );
    }
}
