//! X11: gpui's override-redirect popup, placed in root-window px on the monitor's work area and
//! stacked right above the opener's frame. Wayland: an `xdg_popup` anchored to the box, placed by
//! the compositor (there are no global coordinates). Neither takes the keyboard from the opener.

use gpui::popup::{PopupAnchor, PopupConstraintAdjustment, PopupGravity, PopupOptions};
use gpui::{px, App, Bounds, Pixels, Size, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::windows::Rect;

#[derive(Clone, PartialEq)]
pub(super) enum Place {
    X11 {
        /// The window's frame, root px.
        spot: [i32; 4],
        inset: i32,
        scale: f32,
        /// The opener's top-level window: its frame when a window manager reparented it.
        above: u32,
    },
    Wayland {
        /// In the opener's px.
        anchor: Bounds<Pixels>,
        size: Size<Pixels>,
    },
}

#[derive(Clone)]
pub(super) enum Native {
    X11(u32),
    Wayland,
}

enum Kind {
    X11(u32),
    Wayland,
}

fn kind(window: &Window) -> Option<Kind> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Xcb(h) => Some(Kind::X11(h.window.get())),
        RawWindowHandle::Xlib(h) => Some(Kind::X11(h.window as u32)),
        RawWindowHandle::Wayland(_) => Some(Kind::Wayland),
        _ => None,
    }
}

pub(super) fn locate(
    window: &Window,
    rect: Rect,
    gap: f32,
    size: (f32, f32),
    margin: f32,
) -> Result<Place, &'static str> {
    match kind(window).ok_or("the window is neither X11 nor Wayland")? {
        Kind::X11(owner) => {
            let s = window.scale_factor();
            let found = x11::anchor(owner, s, rect, window.bounds())
                .ok_or("no monitor under the anchor")?;
            Ok(Place::X11 {
                spot: super::frame_in(found.rect, found.work, gap, size, margin, s),
                inset: (margin * s).round() as i32,
                scale: s,
                above: found.top_level,
            })
        }
        Kind::Wayland => {
            // Compositors do not agree on whether a flip mirrors the positioner's offset, so the
            // shadow's room and the gap are folded into the anchor box instead: its bottom is
            // where the surface starts below, its top where it ends above.
            let (x, y, w, h) = rect;
            let below = y + h + gap - margin;
            let above = y - gap + margin;
            let (top, height) = if below > above {
                (above, below - above)
            } else {
                (below - 1.0, 1.0)
            };
            Ok(Place::Wayland {
                anchor: Bounds {
                    origin: gpui::point(px(x - margin), px(top)),
                    size: gpui::size(px(w.max(1.0)), px(height)),
                },
                size: gpui::size(px(size.0 + 2.0 * margin), px(size.1 + 2.0 * margin)),
            })
        }
    }
}

pub(super) fn room(window: &Window, rect: Rect, gap: f32) -> Option<f32> {
    let Kind::X11(owner) = kind(window)? else {
        return None;
    };
    let s = window.scale_factor();
    let found = x11::anchor(owner, s, rect, window.bounds())?;
    Some(super::room_in(found.rect, found.work, gap, s))
}

pub(super) fn options(
    place: &Place,
    owner: gpui::AnyWindowHandle,
    _cx: &App,
) -> (Bounds<Pixels>, gpui::WindowKind, Option<gpui::DisplayId>) {
    match place {
        Place::X11 { spot, scale, .. } => {
            let [x, y, w, h] = spot.map(|v| v as f32 / scale);
            (
                Bounds {
                    origin: gpui::point(px(x), px(y)),
                    size: gpui::size(px(w), px(h)),
                },
                gpui::WindowKind::PopUp,
                None,
            )
        }
        Place::Wayland { anchor, size } => (
            Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: *size,
            },
            gpui::WindowKind::AnchoredPopup(PopupOptions {
                parent: owner,
                anchor_rect: *anchor,
                anchor: PopupAnchor::BottomLeft,
                gravity: PopupGravity::BottomRight,
                constraint_adjustment: PopupConstraintAdjustment::FLIP_Y
                    | PopupConstraintAdjustment::SLIDE_X
                    | PopupConstraintAdjustment::SLIDE_Y,
                offset: gpui::point(px(0.0), px(0.0)),
                // A grab would move the keyboard to the popup and deactivate the opener; as
                // elsewhere the opener keeps it and closes the popup on presses outside it.
                grab: false,
            }),
            None,
        ),
    }
}

pub(super) fn attach(popup: &mut Window, place: &Place) -> Result<Native, &'static str> {
    match place {
        Place::X11 {
            spot, inset, above, ..
        } => {
            let Some(Kind::X11(id)) = kind(popup) else {
                return Err("the popup window has no X11 window");
            };
            if !x11::configure(id, *spot, Some(*above)) {
                return Err("the popup window could not be placed");
            }
            x11::pass_through(id, *spot, *inset);
            popup.show_window(false, || true, |_| {});
            Ok(Native::X11(id))
        }
        Place::Wayland { .. } => Ok(Native::Wayland),
    }
}

pub(super) fn alive(_: &Native) -> bool {
    true
}

/// False when the popup must be opened again: a Wayland popup is not moved after it maps.
pub(super) fn update(
    native: &Native,
    from: &Place,
    to: &Place,
    popup: gpui::AnyWindowHandle,
    cx: &mut App,
) -> bool {
    match (native, from, to) {
        (
            Native::X11(id),
            Place::X11 {
                spot: a, inset: i, ..
            },
            Place::X11 { spot: b, inset, .. },
        ) => {
            if a != b {
                x11::configure(*id, *b, None);
                if a[2..] != b[2..] || i != inset {
                    x11::pass_through(*id, *b, *inset);
                }
            }
            true
        }
        (
            Native::Wayland,
            Place::Wayland { anchor: a, size: s },
            Place::Wayland { anchor, size },
        ) => {
            if a != anchor {
                return false;
            }
            if s != size {
                let size = *size;
                let _ = popup.update(cx, |_, window, _| window.resize(size));
            }
            true
        }
        _ => false,
    }
}

mod x11 {
    use std::cell::{OnceCell, RefCell};

    use gpui::{Bounds, Pixels};
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as _;
    use x11rb::protocol::shape::{self, ConnectionExt as _};
    use x11rb::protocol::xproto::{
        AtomEnum, ClipOrdering, ConfigureWindowAux, ConnectionExt as _, Rectangle, StackMode,
    };
    use x11rb::rust_connection::RustConnection;

    use crate::windows::Rect;

    /// A connection of our own: gpui keeps its own private, and requests about other clients'
    /// windows are the same from any.
    struct Conn {
        c: RustConnection,
        root: u32,
        size: (u16, u16),
        workarea: Option<u32>,
        desktop: Option<u32>,
    }

    #[derive(Clone, Copy)]
    pub(super) struct Found {
        /// The anchor, root px.
        pub rect: Rect,
        pub work: Rect,
        pub top_level: u32,
    }

    type Key = (u32, u32, [u32; 4], Bounds<Pixels>);

    thread_local! {
        static CONN: OnceCell<Option<Conn>> = const { OnceCell::new() };
        /// The last answer: every frame of an open popup asks again, each a few round trips.
        static LAST: RefCell<Option<(Key, Found)>> = const { RefCell::new(None) };
    }

    fn with<R>(f: impl FnOnce(&Conn) -> Option<R>) -> Option<R> {
        CONN.with(|c| c.get_or_init(open).as_ref().and_then(f))
    }

    fn open() -> Option<Conn> {
        let (c, screen) = x11rb::connect(None)
            .map_err(|e| log::warn!("popup windows cannot reach the X server: {e}"))
            .ok()?;
        let s = c.setup().roots.get(screen)?;
        let (root, size) = (s.root, (s.width_in_pixels, s.height_in_pixels));
        let atom = |name: &[u8]| {
            let a = c.intern_atom(true, name).ok()?.reply().ok()?.atom;
            (a != 0).then_some(a)
        };
        let (workarea, desktop) = (atom(b"_NET_WORKAREA"), atom(b"_NET_CURRENT_DESKTOP"));
        Some(Conn {
            c,
            root,
            size,
            workarea,
            desktop,
        })
    }

    fn cardinals(x: &Conn, atom: u32) -> Option<Vec<u32>> {
        let reply =
            x.c.get_property(false, x.root, atom, AtomEnum::CARDINAL, 0, 1024)
                .ok()?
                .reply()
                .ok()?;
        let values = reply.value32()?.collect();
        Some(values)
    }

    fn intersect(a: Rect, b: Rect) -> Option<Rect> {
        let (l, t) = (a.0.max(b.0), a.1.max(b.1));
        let (r, btm) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
        (r > l && btm > t).then_some((l, t, r - l, btm - t))
    }

    fn area(r: Option<Rect>) -> f32 {
        r.map_or(0.0, |r| r.2 * r.3)
    }

    /// `r` (`owner`'s px at `scale`) on the root window, the work area of the monitor showing
    /// most of it, and `owner`'s top-level window.
    pub(super) fn anchor(owner: u32, scale: f32, r: Rect, bounds: Bounds<Pixels>) -> Option<Found> {
        let key = (
            owner,
            scale.to_bits(),
            [r.0, r.1, r.2, r.3].map(f32::to_bits),
            bounds,
        );
        if let Some((k, found)) = LAST.with(|l| *l.borrow()) {
            if k == key {
                return Some(found);
            }
        }
        let found = with(|x| {
            let t =
                x.c.translate_coordinates(
                    owner,
                    x.root,
                    (r.0 * scale).round() as i16,
                    (r.1 * scale).round() as i16,
                )
                .ok()?
                .reply()
                .ok()?;
            let rect = (
                f32::from(t.dst_x),
                f32::from(t.dst_y),
                r.2 * scale,
                r.3 * scale,
            );
            let probe = (rect.0, rect.1, rect.2.max(1.0), rect.3.max(1.0));
            let screen = (0.0, 0.0, f32::from(x.size.0), f32::from(x.size.1));
            let monitor =
                x.c.randr_get_monitors(x.root, true)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .and_then(|m| {
                        m.monitors
                            .iter()
                            .map(|m| {
                                (
                                    f32::from(m.x),
                                    f32::from(m.y),
                                    f32::from(m.width),
                                    f32::from(m.height),
                                )
                            })
                            .max_by(|a, b| {
                                area(intersect(*a, probe)).total_cmp(&area(intersect(*b, probe)))
                            })
                    })
                    .unwrap_or(screen);
            let desktop = x
                .desktop
                .and_then(|a| cardinals(x, a))
                .and_then(|d| d.first().copied())
                .unwrap_or(0) as usize;
            let work = x
                .workarea
                .and_then(|a| cardinals(x, a))
                .and_then(|w| {
                    let w = w.get(desktop * 4..desktop * 4 + 4)?;
                    intersect(
                        (w[0] as f32, w[1] as f32, w[2] as f32, w[3] as f32),
                        monitor,
                    )
                })
                .unwrap_or(monitor);
            let mut top_level = owner;
            loop {
                let tree = x.c.query_tree(top_level).ok()?.reply().ok()?;
                if tree.parent == tree.root || tree.parent == 0 {
                    break;
                }
                top_level = tree.parent;
            }
            Some(Found {
                rect,
                work,
                top_level,
            })
        })?;
        LAST.with(|l| *l.borrow_mut() = Some((key, found)));
        Some(found)
    }

    /// Moves `popup` to `spot`; with `above`, also stacks it right above that window.
    pub(super) fn configure(popup: u32, [x, y, w, h]: [i32; 4], above: Option<u32>) -> bool {
        with(|c| {
            let frame = ConfigureWindowAux::new()
                .x(x)
                .y(y)
                .width(w.max(1) as u32)
                .height(h.max(1) as u32);
            let stacked = above.is_some_and(|a| {
                let aux = frame.sibling(a).stack_mode(StackMode::ABOVE);
                c.c.configure_window(popup, &aux)
                    .ok()
                    .is_some_and(|r| r.check().is_ok())
            });
            // Not a sibling of the opener's frame (an unusual window manager): on top instead.
            Some(
                stacked
                    || c.c
                        .configure_window(popup, &frame)
                        .ok()
                        .is_some_and(|r| r.check().is_ok()),
            )
        })
        .unwrap_or(false)
    }

    /// The shadow's room takes no input: presses there reach the window under it.
    pub(super) fn pass_through(popup: u32, [_, _, w, h]: [i32; 4], inset: i32) {
        if inset <= 0 {
            return;
        }
        let _ = with(|c| {
            let rect = Rectangle {
                x: inset as i16,
                y: inset as i16,
                width: (w - 2 * inset).max(1) as u16,
                height: (h - 2 * inset).max(1) as u16,
            };
            c.c.shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                ClipOrdering::UNSORTED,
                popup,
                0,
                0,
                &[rect],
            )
            .ok()?;
            c.c.flush().ok()
        });
    }
}
