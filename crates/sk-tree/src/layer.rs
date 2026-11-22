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
}

/// Borrowed for the call that hands them over: `AddRef` (clone a borrowed interface) to keep them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Composition {
    /// The window's `IDCompositionDevice`. A visual belongs to the device that made it.
    pub device: NonNull<c_void>,
    /// The `IDCompositionVisual` drawn under the page: add yours to it.
    pub underlay: NonNull<c_void>,
}

/// A native layer under one `<sk-layer>` element. Its methods run on the UI thread inside the
/// page's paint: return quickly and do not call back into Skathi. Dropped when the element goes.
pub trait Layer {
    /// The element is painted in `host` for the first time, in another window, or after the
    /// window rebuilt what it composes with: move the layer there. A placement follows.
    fn attach(&mut self, host: &LayerHost);
    /// Show the layer at `placement`, in device pixels of the window, or hide it (`None`: the
    /// element was not painted in the last frame).
    fn place(&mut self, placement: Option<Placement>);
}

#[derive(Clone, Copy, PartialEq)]
struct Home {
    window: WinKey,
    handle: Option<RawWindowHandle>,
    composition: Option<Composition>,
}

struct Live {
    name: String,
    layer: Box<dyn Layer>,
    home: Option<Home>,
    /// Keeps `home.composition` alive while the layer may use it.
    _held: Option<Held>,
    placed: Option<Placement>,
}

type Make = Rc<dyn Fn() -> Box<dyn Layer>>;

thread_local! {
    static MAKERS: RefCell<HashMap<String, Make>> = RefCell::new(HashMap::new());
    static LAYERS: RefCell<HashMap<u32, Live>> = RefCell::new(HashMap::new());
    static PAINTED: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static DREW: RefCell<Vec<WinKey>> = const { RefCell::new(Vec::new()) };
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

pub(crate) fn drawing(win: WinKey) {
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
    let home = Home {
        window: crate::windows::key(window),
        handle: handle_of(window),
        composition,
    };
    if !settle(id, name, home, held, placement) {
        crate::render::warn_compat(
            &format!("sk-layer:{name}"),
            &format!("<sk-layer name=\"{name}\"> shows nothing: the embedding program registered no layer of that name"),
        );
    }
}

/// Returns whether the element has a layer.
fn settle(id: u32, name: &str, home: Home, held: Option<Held>, placement: Placement) -> bool {
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
        let live = Live {
            name: name.to_string(),
            layer: make(),
            home: None,
            _held: None,
            placed: None,
        };
        LAYERS.with(|l| l.borrow_mut().insert(id, live));
    }
    let known = LAYERS.with(|l| {
        let mut layers = l.borrow_mut();
        let Some(live) = layers.get_mut(&id) else {
            return false;
        };
        if live.home != Some(home) {
            live._held = held;
            live.home = Some(home);
            live.placed = None;
            live.layer.attach(&LayerHost {
                window: home.window,
                handle: home.handle,
                composition: home.composition,
            });
        }
        if live.placed != Some(placement) {
            live.placed = Some(placement);
            live.layer.place(Some(placement));
        }
        true
    });
    if known {
        PAINTED.with(|p| p.borrow_mut().insert(id));
    }
    known
}

/// Hides each layer of `win` that its last paint did not reach.
pub(crate) fn tick(win: WinKey) {
    let drew = DREW.with(|d| {
        let mut d = d.borrow_mut();
        let before = d.len();
        d.retain(|w| *w != win);
        d.len() != before
    });
    if !drew {
        return;
    }
    let of = |live: &Live| live.home.map(|h| h.window);
    LAYERS.with(|l| {
        PAINTED.with(|p| {
            let mut painted = p.borrow_mut();
            for (id, live) in l.borrow_mut().iter_mut() {
                if of(live) != Some(win) {
                    continue;
                }
                if !painted.remove(id) && live.placed.take().is_some() {
                    live.layer.place(None);
                }
            }
        });
    });
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
            self.0.borrow_mut().push(format!("attach {}", host.window));
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

    fn paint(id: u32, window: WinKey, placement: Placement) -> bool {
        let home = Home {
            window,
            handle: None,
            composition: None,
        };
        settle(id, "player", home, None, placement)
    }

    #[test]
    fn a_layer_is_placed_once_per_change_hidden_when_not_painted_and_dropped_with_its_window() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let take = |log: &Rc<RefCell<Vec<String>>>| std::mem::take(&mut *log.borrow_mut());
        assert!(!paint(7, 1, at(0.0)), "no layer of that name yet");
        let made = log.clone();
        register("player", move || Box::new(Log(made.clone())));
        paint(7, 1, at(0.0));
        drawing(1);
        paint(7, 1, at(0.0));
        tick(1);
        assert_eq!(take(&log), ["attach 1", "place 0"]);

        drawing(1);
        paint(7, 1, at(5.0));
        tick(1);
        assert_eq!(take(&log), ["place 5"]);

        drawing(1);
        tick(1);
        assert_eq!(take(&log), ["hide"], "not painted in a draw of its window");
        tick(1);
        assert!(take(&log).is_empty(), "no draw, no news");

        drawing(2);
        paint(7, 2, at(5.0));
        tick(2);
        assert_eq!(
            take(&log),
            ["attach 2", "place 5"],
            "it follows its element"
        );

        forget_window(2);
        assert_eq!(take(&log), ["drop"]);
    }
}
