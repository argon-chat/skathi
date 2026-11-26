//! `<sk-layer name="…">`: a native layer of the embedding program, shown under the page. The element
//! is a transparent box; its layer is told where the box is from the paint that draws it, and which
//! window it is in, so the layer moves in the frame the page does.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use gpui::Window;
use raw_window_handle::RawWindowHandle;

pub use crate::iframe::Placement;
use crate::windows::WinKey;

/// Where a layer goes: the window its element is painted in.
#[derive(Clone, Copy, Debug)]
pub struct LayerHost {
    /// The window's Skathi ID.
    pub window: u64,
    /// The window: an `HWND`, on macOS the `NSView` the page is drawn in.
    pub handle: Option<RawWindowHandle>,
    /// Windows: where to add a visual under the page. `None` when the window does not present
    /// through DirectComposition, where nothing can be shown under it.
    pub composition: Option<Composition>,
    /// macOS: the `NSWindow` the page's view is in now, which is not Skathi's when the embedding
    /// program moved the view (or its content view) into a window of its own.
    pub native_window: Option<NonNull<c_void>>,
}

/// Borrowed for the call that hands them over: `AddRef` (clone a borrowed interface) to keep them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Composition {
    /// The window's `IDCompositionDevice`. A visual belongs to the device that made it.
    pub device: NonNull<c_void>,
    /// The `IDCompositionVisual` drawn under the page: add yours to it.
    pub underlay: NonNull<c_void>,
}

/// A page frame of a layer's window, as [`Layer::frame_submitted`] hears of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LayerFrame {
    /// The page frame's number in its window, from 1, one more each frame.
    pub sequence: u64,
    /// When it was handed to the platform, in [`crate::surface::monotonic_ns`]'s clock: a submit
    /// time, not when the display shows it.
    pub submitted_at: u64,
    /// The frame ID of this layer's picture composited by a backdrop in this page frame.
    /// Repeats while the same picture is used. `None` when no picture was composited.
    pub picture: Option<u64>,
    /// The platform's target display time for this page frame, in [`crate::surface::monotonic_ns`]'s
    /// clock. The macOS Metal display link supplies one for its drawable.
    pub target_present_at: Option<u64>,
}

/// A native layer under one `<sk-layer>` element. Its methods run on the UI thread inside the
/// page's paint or present: return quickly and do not call back into Skathi. Dropped when the
/// element goes.
pub trait Layer {
    /// The element is painted in `host` for the first time, in another window (or its view moved
    /// to another native window or superview), or after the window rebuilt what it composes
    /// with: move the layer there. A placement follows.
    fn attach(&mut self, host: &LayerHost);
    /// Show the layer at `placement`, in device pixels of the window, or hide it (`None`: the
    /// element was not painted in the last frame).
    fn place(&mut self, placement: Option<Placement>);
    /// Windows: Skathi committed the composition device right after presenting the frame that
    /// carried the last `attach` or `place`, so those changes are on screen with it. Only after
    /// such a frame; [`Self::frame_submitted`] hears of every frame.
    fn presented(&mut self) {}
    /// The surface whose frames are this layer's picture, asked once when the layer is made. With
    /// one, backdrop effects and `backdrop-filter` over the element see the picture where the
    /// layer is, rounded and clipped like it; it is never drawn into the page itself. Publish to
    /// it as to an `<sk-surface>`, while its `info().visible` says a backdrop reads it.
    fn picture(&mut self) -> Option<crate::surface::SurfaceHandle> {
        None
    }
    /// Its place among the layers of its window in paint order, 0 the bottom: a later-painted
    /// `<sk-layer>` lies over an earlier one, in backdrops too. Called before `place` when it changes.
    fn stack(&mut self, _index: u32) {}
    /// A page frame of its window was handed to the platform (presented to the swap chain, or
    /// committed to the layer tree) while the layer is placed: every frame, on every platform,
    /// right after the window's present call returns.
    fn frame_submitted(&mut self, _frame: LayerFrame) {}
}

#[derive(Clone, Copy, PartialEq)]
struct Home {
    window: WinKey,
    handle: Option<RawWindowHandle>,
    composition: Option<Composition>,
    /// macOS: the page's view's `NSWindow` and superview.
    container: [usize; 2],
}

struct Live {
    name: String,
    layer: Box<dyn Layer>,
    home: Option<Home>,
    /// Keeps `home.composition` alive while the layer may use it.
    _held: Option<Held>,
    placed: Option<Placement>,
    /// Told something since the last commit of its window's composition device.
    awaiting: bool,
    picture: Option<crate::surface::SurfaceHandle>,
    stacked: Option<u32>,
}

type Make = Rc<dyn Fn() -> Box<dyn Layer>>;

thread_local! {
    static MAKERS: RefCell<HashMap<String, Make>> = RefCell::new(HashMap::new());
    static LAYERS: RefCell<HashMap<u32, Live>> = RefCell::new(HashMap::new());
    static PAINTED: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static DREW: RefCell<Vec<WinKey>> = const { RefCell::new(Vec::new()) };
    static HOOKED: RefCell<HashSet<WinKey>> = RefCell::new(HashSet::new());
    static SUBMIT_HOOKED: RefCell<HashSet<WinKey>> = RefCell::new(HashSet::new());
    /// Layers painted so far in each window's frame.
    static STACKED: RefCell<HashMap<WinKey, u32>> = RefCell::new(HashMap::new());
}

/// Has every `<sk-layer name="…">` of that name show a layer made by `make`: one per element.
/// On the UI thread; a later call for the same name replaces the earlier for new elements.
pub fn register(name: &str, make: impl Fn() -> Box<dyn Layer> + 'static) {
    MAKERS.with(|m| m.borrow_mut().insert(name.to_string(), Rc::new(make)));
}

#[cfg(windows)]
struct Held(::windows::core::IUnknown, ::windows::core::IUnknown);
#[cfg(not(windows))]
struct Held;

#[cfg(not(windows))]
fn composition(_: &Window) -> Option<(Composition, Held)> {
    None
}

#[cfg(windows)]
fn composition(window: &Window) -> Option<(Composition, Held)> {
    use ::windows::core::{IUnknown, Interface};
    let (device, underlay) = window.vn_composition_underlay()?;
    // SAFETY: each is an owned COM reference handed over by the renderer.
    let held = unsafe {
        Held(
            IUnknown::from_raw(device as *mut c_void),
            IUnknown::from_raw(underlay as *mut c_void),
        )
    };
    let pair = Composition {
        device: NonNull::new(held.0.as_raw())?,
        underlay: NonNull::new(held.1.as_raw())?,
    };
    Some((pair, held))
}

fn handle_of(window: &Window) -> Option<RawWindowHandle> {
    use raw_window_handle::HasWindowHandle;
    Some(HasWindowHandle::window_handle(window).ok()?.as_raw())
}

/// The embedding program may move the page's view into a window of its own, which Skathi's
/// window key does not see.
#[cfg(target_os = "macos")]
fn container_of(handle: Option<RawWindowHandle>) -> [usize; 2] {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let Some(RawWindowHandle::AppKit(h)) = handle else {
        return [0; 2];
    };
    let view = h.ns_view.as_ptr().cast::<AnyObject>();
    // SAFETY: the view of a window that is painting, on the main thread.
    unsafe {
        let window: *mut AnyObject = msg_send![&*view, window];
        let superview: *mut AnyObject = msg_send![&*view, superview];
        [window as usize, superview as usize]
    }
}

#[cfg(not(target_os = "macos"))]
fn container_of(_: Option<RawWindowHandle>) -> [usize; 2] {
    [0; 2]
}

fn hook(window: &Window, win: WinKey) {
    if HOOKED.with(|h| h.borrow_mut().insert(win)) {
        window.vn_on_composition_commit(Box::new(move || committed(win)));
    }
}

fn hook_submits(window: &mut Window, win: WinKey) {
    if SUBMIT_HOOKED.with(|h| h.borrow_mut().insert(win)) {
        window.vn_on_frame_submission(Box::new(move |frame| submitted(win, frame)));
    }
}

fn submitted(win: WinKey, frame: gpui::VnSubmittedFrame<'_>) {
    let submitted_at = crate::surface::monotonic_ns();
    LAYERS.with(|l| {
        let Ok(mut layers) = l.try_borrow_mut() else {
            return;
        };
        for (&id, live) in layers.iter_mut() {
            if live.placed.is_some() && live.home.is_some_and(|h| h.window == win) {
                let picture = frame
                    .pictures
                    .iter()
                    .find(|(key, _)| *key == u64::from(id))
                    .map(|(_, picture)| *picture);
                live.layer.frame_submitted(LayerFrame {
                    sequence: frame.sequence,
                    submitted_at,
                    picture,
                    target_present_at: frame.target_present_at,
                });
            }
        }
    });
}

/// The surface `<sk-layer>` `id`'s layer named as its picture.
pub(crate) fn picture(id: u32) -> Option<crate::surface::SurfaceHandle> {
    LAYERS.with(|l| l.borrow().get(&id)?.picture.clone())
}

fn committed(win: WinKey) {
    LAYERS.with(|l| {
        for live in l.borrow_mut().values_mut() {
            if live.awaiting && live.home.is_some_and(|h| h.window == win) {
                live.awaiting = false;
                live.layer.presented();
            }
        }
    });
}

pub(crate) fn drawing(win: WinKey) {
    STACKED.with(|s| s.borrow_mut().remove(&win));
    if LAYERS.with(|l| l.borrow().is_empty()) {
        return;
    }
    DREW.with(|d| {
        let mut d = d.borrow_mut();
        if !d.contains(&win) {
            d.push(win);
        }
    });
}

/// The element is painted in `window` at `placement`: its layer follows at once.
pub(crate) fn painted(id: u32, name: &str, placement: Placement, window: &mut Window) {
    let (composition, held) = composition(window).unzip();
    let handle = handle_of(window);
    let home = Home {
        window: crate::windows::key(window),
        handle,
        composition,
        container: container_of(handle),
    };
    let index = STACKED.with(|s| {
        let mut s = s.borrow_mut();
        let next = s.entry(home.window).or_insert(0);
        *next += 1;
        *next - 1
    });
    let told = settle(id, name, home, held, placement, index);
    if told.is_some() {
        hook_submits(window, home.window);
    }
    match told {
        None => crate::render::warn_compat(
            &format!("sk-layer:{name}"),
            &format!("<sk-layer name=\"{name}\"> shows nothing: the embedding program registered no layer of that name"),
        ),
        // The page's frame is not presented yet: what the layer changed goes out right after it.
        Some(true) if home.composition.is_some() => {
            hook(window, home.window);
            window.vn_composition_commit(true);
        }
        Some(_) => {}
    }
}

/// Returns whether the element has a layer, and if so whether it was told anything.
fn settle(
    id: u32,
    name: &str,
    home: Home,
    held: Option<Held>,
    placement: Placement,
    index: u32,
) -> Option<bool> {
    let make = LAYERS.with(|l| {
        let mut layers = l.borrow_mut();
        if layers.get(&id).is_some_and(|live| live.name != name) {
            layers.remove(&id);
        }
        if layers.contains_key(&id) {
            return None;
        }
        MAKERS.with(|m| m.borrow().get(name).cloned())
    });
    if let Some(make) = make {
        let mut layer = make();
        let live = Live {
            name: name.to_string(),
            picture: layer.picture(),
            layer,
            home: None,
            _held: None,
            placed: None,
            awaiting: false,
            stacked: None,
        };
        LAYERS.with(|l| l.borrow_mut().insert(id, live));
    }
    let told = LAYERS.with(|l| {
        let mut layers = l.borrow_mut();
        let live = layers.get_mut(&id)?;
        let mut told = false;
        if live.home != Some(home) {
            live._held = held;
            live.home = Some(home);
            live.placed = None;
            live.stacked = None;
            live.layer.attach(&LayerHost {
                window: home.window,
                handle: home.handle,
                composition: home.composition,
                native_window: NonNull::new(home.container[0] as *mut c_void),
            });
            told = true;
        }
        if live.stacked != Some(index) {
            live.stacked = Some(index);
            live.layer.stack(index);
            told = true;
        }
        if live.placed != Some(placement) {
            live.placed = Some(placement);
            live.layer.place(Some(placement));
            told = true;
        }
        live.awaiting |= told && home.composition.is_some();
        Some(told)
    });
    if told.is_some() {
        PAINTED.with(|p| p.borrow_mut().insert(id));
    }
    told
}

/// Hides each layer of `window` that its last paint did not reach. That frame is on screen
/// already, so a hide is committed at once.
pub(crate) fn tick(window: &Window) {
    if hide_unpainted(crate::windows::key(window)) {
        window.vn_composition_commit(false);
    }
}

/// Returns whether a layer that shows through composition was hidden.
fn hide_unpainted(win: WinKey) -> bool {
    let drew = DREW.with(|d| {
        let mut d = d.borrow_mut();
        let before = d.len();
        d.retain(|w| *w != win);
        d.len() != before
    });
    if !drew {
        return false;
    }
    let mut composed = false;
    LAYERS.with(|l| {
        PAINTED.with(|p| {
            let mut painted = p.borrow_mut();
            for (id, live) in l.borrow_mut().iter_mut() {
                let Some(home) = live.home.filter(|h| h.window == win) else {
                    continue;
                };
                if !painted.remove(id) && live.placed.take().is_some() {
                    live.layer.place(None);
                    if home.composition.is_some() {
                        live.awaiting = true;
                        composed = true;
                    }
                }
            }
        });
    });
    composed
}

pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    drop(LAYERS.with(|l| {
        let mut l = l.borrow_mut();
        let gone: Vec<u32> = l.keys().copied().filter(|&id| !live(id)).collect();
        gone.into_iter()
            .filter_map(|id| l.remove(&id))
            .collect::<Vec<_>>()
    }));
    PAINTED.with(|p| p.borrow_mut().retain(|&id| live(id)));
}

/// A window closed: its layers go, and their elements get new ones if painted again elsewhere.
pub(crate) fn forget_window(win: WinKey) {
    drop(LAYERS.with(|l| {
        let mut l = l.borrow_mut();
        let ids: Vec<u32> = l
            .iter()
            .filter(|(_, live)| live.home.is_some_and(|h| h.window == win))
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter()
            .filter_map(|id| l.remove(&id))
            .collect::<Vec<_>>()
    }));
    DREW.with(|d| d.borrow_mut().retain(|w| *w != win));
    HOOKED.with(|h| h.borrow_mut().remove(&win));
    SUBMIT_HOOKED.with(|h| h.borrow_mut().remove(&win));
    STACKED.with(|s| s.borrow_mut().remove(&win));
}

pub fn shutdown() {
    drop(LAYERS.with(|l| std::mem::take(&mut *l.borrow_mut())));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Log(Rc<RefCell<Vec<String>>>);

    impl Layer for Log {
        fn attach(&mut self, host: &LayerHost) {
            let told = match host.native_window {
                Some(native) => format!("attach {} in {:x}", host.window, native.as_ptr() as usize),
                None => format!("attach {}", host.window),
            };
            self.0.borrow_mut().push(told);
        }
        fn place(&mut self, placement: Option<Placement>) {
            let at = placement.map_or("hide".to_string(), |p| format!("place {}", p.bounds[0]));
            self.0.borrow_mut().push(at);
        }
    }

    impl Drop for Log {
        fn drop(&mut self) {
            self.0.borrow_mut().push("drop".into());
        }
    }

    fn at(x: f32) -> Placement {
        Placement {
            bounds: [x, 0.0, 10.0, 10.0],
            clip: [x, 0.0, 10.0, 10.0],
            radii: [0.0; 4],
            scale: 1.0,
            opacity: 1.0,
        }
    }

    fn paint_in(
        id: u32,
        window: WinKey,
        container: [usize; 2],
        placement: Placement,
    ) -> Option<bool> {
        let home = Home {
            window,
            handle: None,
            composition: None,
            container,
        };
        settle(id, "player", home, None, placement, 0)
    }

    fn paint(id: u32, window: WinKey, placement: Placement) -> Option<bool> {
        paint_in(id, window, [0; 2], placement)
    }

    struct Stacked(Rc<RefCell<Vec<String>>>, u32);

    impl Layer for Stacked {
        fn attach(&mut self, _: &LayerHost) {}
        fn place(&mut self, placement: Option<Placement>) {
            let at = placement.map_or("hide".to_string(), |p| format!("place {}", p.bounds[0]));
            self.0.borrow_mut().push(format!("{} {at}", self.1));
        }
        fn stack(&mut self, index: u32) {
            self.0
                .borrow_mut()
                .push(format!("{} stack {index}", self.1));
        }
        fn frame_submitted(&mut self, frame: LayerFrame) {
            self.0.borrow_mut().push(format!(
                "{} frame {} picture {:?} target {:?}",
                self.1, frame.sequence, frame.picture, frame.target_present_at
            ));
        }
    }

    #[test]
    fn layers_hear_their_paint_order_before_their_place_and_every_frame_of_their_window() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let take = |log: &Rc<RefCell<Vec<String>>>| std::mem::take(&mut *log.borrow_mut());
        let (made, next) = (log.clone(), Rc::new(std::cell::Cell::new(0)));
        register("stacked", move || {
            next.set(next.get() + 1);
            Box::new(Stacked(made.clone(), next.get()))
        });
        let home = |window| Home {
            window,
            handle: None,
            composition: None,
            container: [0; 2],
        };
        let paint = |id, window, x, index| settle(id, "stacked", home(window), None, at(x), index);
        paint(21, 1, 0.0, 0);
        paint(22, 1, 1.0, 1);
        paint(23, 2, 2.0, 0);
        assert_eq!(
            take(&log),
            [
                "1 stack 0",
                "1 place 0",
                "2 stack 1",
                "2 place 1",
                "3 stack 0",
                "3 place 2"
            ]
        );
        paint(22, 1, 1.0, 0);
        paint(21, 1, 0.0, 1);
        assert_eq!(take(&log), ["2 stack 0", "1 stack 1"], "the order swapped");
        submitted(
            1,
            gpui::VnSubmittedFrame {
                sequence: 9,
                pictures: &[(21, 117), (23, 130)],
                target_present_at: Some(999),
            },
        );
        let mut heard = take(&log);
        heard.sort();
        assert_eq!(
            heard,
            [
                "1 frame 9 picture Some(117) target Some(999)",
                "2 frame 9 picture None target Some(999)"
            ],
            "window 2's layer hears nothing"
        );
        forget_window(1);
        forget_window(2);
    }

    #[test]
    fn a_layer_is_attached_again_when_its_view_moves_to_another_native_window_or_superview() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let take = |log: &Rc<RefCell<Vec<String>>>| std::mem::take(&mut *log.borrow_mut());
        let made = log.clone();
        register("player", move || Box::new(Log(made.clone())));
        let (skathi, panel, content) = (0xa0, 0xb0, 0xc0);

        paint_in(3, 1, [skathi, content], at(0.0));
        assert_eq!(take(&log), ["attach 1 in a0", "place 0"]);
        assert_eq!(paint_in(3, 1, [skathi, content], at(0.0)), Some(false));
        assert!(take(&log).is_empty());

        paint_in(3, 1, [panel, content], at(0.0));
        assert_eq!(
            take(&log),
            ["attach 1 in b0", "place 0"],
            "the embedding program moved the content view into its panel"
        );

        paint_in(3, 1, [panel, 0xd0], at(0.0));
        assert_eq!(
            take(&log),
            ["attach 1 in b0", "place 0"],
            "another superview"
        );

        paint_in(3, 1, [skathi, content], at(4.0));
        assert_eq!(take(&log), ["attach 1 in a0", "place 4"], "and back");
    }

    #[test]
    fn a_layer_is_placed_once_per_change_hidden_when_not_painted_and_dropped_with_its_window() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let take = |log: &Rc<RefCell<Vec<String>>>| std::mem::take(&mut *log.borrow_mut());
        assert!(paint(7, 1, at(0.0)).is_none(), "no layer of that name yet");
        let made = log.clone();
        register("player", move || Box::new(Log(made.clone())));
        paint(7, 1, at(0.0));
        drawing(1);
        paint(7, 1, at(0.0));
        hide_unpainted(1);
        assert_eq!(take(&log), ["attach 1", "place 0"]);

        drawing(1);
        paint(7, 1, at(5.0));
        hide_unpainted(1);
        assert_eq!(take(&log), ["place 5"]);

        drawing(1);
        hide_unpainted(1);
        assert_eq!(take(&log), ["hide"], "not painted in a draw of its window");
        hide_unpainted(1);
        assert!(take(&log).is_empty(), "no draw, no news");

        drawing(2);
        paint(7, 2, at(5.0));
        hide_unpainted(2);
        assert_eq!(
            take(&log),
            ["attach 2", "place 5"],
            "it follows its element"
        );

        forget_window(2);
        assert_eq!(take(&log), ["drop"]);
    }
}
