//! macOS: gpui's non-activating panel, a child window right above the opener, placed in screen
//! points on the visible frame of the screen under the anchor. AppKit's screen space is flipped
//! here (`y` grows down) so `place` works in it as in device px elsewhere.

use gpui::{px, App, Bounds, Pixels, Window};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::windows::Rect;

const NS_WINDOW_ABOVE: isize = 1;

#[derive(Clone, PartialEq)]
pub(super) struct Place {
    /// The panel's frame, flipped screen points.
    frame: [i32; 4],
    scale: f32,
    /// The opener's `NSWindow`.
    owner: usize,
}

#[derive(Clone)]
pub(super) struct Native(usize);

pub(super) fn locate(
    window: &Window,
    rect: Rect,
    gap: f32,
    size: (f32, f32),
    margin: f32,
    placement: super::Placement,
) -> Result<(Place, Option<Rect>), &'static str> {
    let (anchor, work, owner) = anchor(window, rect).ok_or("no screen under the anchor")?;
    let s = window.scale_factor();
    let device = |r: Rect| (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
    let anchor = device(anchor);
    let frame = super::frame_in(placement, anchor, device(work), gap, size, margin, s);
    let bounds = super::in_owner(frame, rect, anchor, s);
    Ok((
        Place {
            frame,
            scale: s,
            owner: owner as usize,
        },
        Some(bounds),
    ))
}

pub(super) fn room(window: &Window, rect: Rect, gap: f32) -> Option<f32> {
    let (r, work, _) = anchor(window, rect)?;
    Some(super::room_in(r, work, gap, 1.0))
}

pub(super) fn options(
    place: &Place,
    _owner: gpui::AnyWindowHandle,
    _cx: &App,
) -> (Bounds<Pixels>, gpui::WindowKind, Option<gpui::DisplayId>) {
    let [_, _, w, h] = place.frame.map(|v| v as f32 / place.scale);
    (
        Bounds {
            origin: gpui::point(px(0.0), px(0.0)),
            size: gpui::size(px(w), px(h)),
        },
        gpui::WindowKind::PopUp,
        None,
    )
}

pub(super) fn attach(
    popup: &mut Window,
    place: &Place,
    menu: bool,
) -> Result<Native, &'static str> {
    let panel = ns_window(popup).ok_or("the popup window has no NSWindow")?;
    let owner = place.owner as *mut AnyObject;
    // SAFETY: both are live windows (the opener painted this frame), used on the main thread.
    unsafe {
        // A press in the panel leaves the opener key: it keeps the keyboard and stays active.
        let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: true];
        let _: () = msg_send![panel, setHasShadow: menu];
        if menu {
            let content: *mut AnyObject = msg_send![panel, contentView];
            let _: () = msg_send![content, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![content, layer];
            let _: () = msg_send![layer, setCornerRadius: 8.0_f64];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
        let level: isize = msg_send![owner, level];
        let _: () = msg_send![panel, setLevel: level];
        let _: () = msg_send![panel, setFrame: frame(place), display: false];
        let _: () = msg_send![owner, addChildWindow: panel, ordered: NS_WINDOW_ABOVE];
        let number: isize = msg_send![owner, windowNumber];
        let _: () = msg_send![panel, orderWindow: NS_WINDOW_ABOVE, relativeTo: number];
    }
    // Under other windows (a quiet test host) AppKit would stop its frames.
    popup.set_draws_while_occluded(true);
    Ok(Native(panel as usize))
}

pub(super) fn alive(_: &Native) -> bool {
    true
}

pub(super) fn update(
    native: &Native,
    from: &Place,
    to: &Place,
    _popup: gpui::AnyWindowHandle,
    _cx: &mut App,
) -> bool {
    if from.frame != to.frame {
        let panel = native.0 as *mut AnyObject;
        // SAFETY: the host's panel is open (closing it drops the host first).
        unsafe {
            let _: () = msg_send![panel, setFrame: frame(to), display: true];
        }
    }
    true
}

fn frame(place: &Place) -> CGRect {
    let [x, y, w, h] = place.frame.map(|v| f64::from(v) / f64::from(place.scale));
    CGRect::new(CGPoint::new(x, -(y + h)), CGSize::new(w, h))
}

fn ns_window(window: &Window) -> Option<*mut AnyObject> {
    let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).ok()?.as_raw() else {
        return None;
    };
    let view = h.ns_view.as_ptr().cast::<AnyObject>();
    // SAFETY: the view lives as long as its window.
    let w: *mut AnyObject = unsafe { msg_send![&*view, window] };
    (!w.is_null()).then_some(w)
}

fn flip(r: CGRect) -> Rect {
    (
        r.origin.x as f32,
        -(r.origin.y + r.size.height) as f32,
        r.size.width as f32,
        r.size.height as f32,
    )
}

fn overlap(a: CGRect, b: CGRect) -> f64 {
    let w = (a.origin.x + a.size.width).min(b.origin.x + b.size.width) - a.origin.x.max(b.origin.x);
    let h =
        (a.origin.y + a.size.height).min(b.origin.y + b.size.height) - a.origin.y.max(b.origin.y);
    w.max(0.0) * h.max(0.0)
}

/// `r` (`window`'s content points) on the screen, the visible frame of the screen showing most of
/// it (or the window's), both flipped, and the window's `NSWindow`.
fn anchor(window: &Window, r: Rect) -> Option<(Rect, Rect, *mut AnyObject)> {
    let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).ok()?.as_raw() else {
        return None;
    };
    let view = h.ns_view.as_ptr().cast::<AnyObject>();
    // SAFETY: AppKit geometry getters on the window's own view and screens, on the main thread.
    unsafe {
        let view = &*view;
        let ns_window: *mut AnyObject = msg_send![view, window];
        let ns_window = ns_window.as_ref()?;
        let flipped: bool = msg_send![view, isFlipped];
        let bounds: CGRect = msg_send![view, bounds];
        let (x, y, w, h) = (
            f64::from(r.0),
            f64::from(r.1),
            f64::from(r.2),
            f64::from(r.3),
        );
        let y = if flipped {
            y
        } else {
            bounds.size.height - y - h
        };
        let local = CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
        let in_window: CGRect =
            msg_send![view, convertRect: local, toView: std::ptr::null_mut::<AnyObject>()];
        let on_screen: CGRect = msg_send![ns_window, convertRectToScreen: in_window];
        let probe = CGRect::new(
            on_screen.origin,
            CGSize::new(
                on_screen.size.width.max(1.0),
                on_screen.size.height.max(1.0),
            ),
        );
        let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
        let screens = screens.as_ref()?;
        let count: usize = msg_send![screens, count];
        let mut best: Option<(f64, *mut AnyObject)> = None;
        for i in 0..count {
            let screen: *mut AnyObject = msg_send![screens, objectAtIndex: i];
            let Some(screen) = screen.as_ref() else {
                continue;
            };
            let f: CGRect = msg_send![screen, frame];
            let area = overlap(f, probe);
            if area > 0.0 && best.is_none_or(|(a, _)| area > a) {
                best = Some((area, screen as *const _ as *mut AnyObject));
            }
        }
        let screen = match best {
            Some((_, s)) => s,
            None => msg_send![ns_window, screen],
        };
        let screen = screen.as_ref()?;
        let work: CGRect = msg_send![screen, visibleFrame];
        Some((
            flip(on_screen),
            flip(work),
            ns_window as *const _ as *mut AnyObject,
        ))
    }
}
