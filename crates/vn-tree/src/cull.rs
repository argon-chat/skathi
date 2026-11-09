//! Off-screen culling — `content-visibility: auto` for clipped flows.
//!
//! A scroll container keeps every row in the DOM, and gpui rebuilds, lays out,
//! prepaints and paints every one of them on every drawn frame. The retained
//! cache ([`crate::cache`]) answers that for frames where nothing moves, but a
//! scroll moves every row: each one re-renders, and a 300-row list pays for all
//! 300 to show the twenty in its viewport.
//!
//! A subtree whose box lies wholly outside every clip around it — the window,
//! and each ancestor whose `overflow` is not `visible` — paints nothing. Here it
//! is lowered as an empty box of the size it last laid out to, carrying the
//! margin and alignment its parent's layout reads, exactly like a cache
//! boundary's wrapper. Its descendants' rectangles are carried forward,
//! translated with it, so `getBoundingClientRect()`, observers and devtools
//! still see where every node is.
//!
//! ## When the box may stand in for the subtree
//!
//! The pin is the cache's: the box is only trusted while nothing can have
//! resized it — the subtree's `sub_rev`, the inherited context threaded down
//! the build, and the parent's size are unchanged, and the box measured the
//! same size on the two frames before it was first pinned.
//!
//! The parent's layout must not see through the box: a flex or grid parent
//! (no margin collapsing), or a node that is itself a flex/grid container or
//! clips its overflow (its children's margins cannot collapse through it).
//! A block box in block flow is lowered normally.
//!
//! A subtree is refused when its content can be seen or heard outside the box:
//! out-of-flow and deferred boxes, relative insets, a focused or captured node,
//! a moving animation, live widgets (`<input>`, `<textarea>`, `<video>`,
//! `<canvas>`, `<iframe>`), group anchors and pop-outs — and everything while a
//! screen reader is attached, which reads off-screen content too.
//!
//! ## Where the clip is
//!
//! Rectangles are last frame's, and the build runs before this frame's layout,
//! so each clip is predicted: a scroll container's children move by the offset
//! its `ScrollHandle` moved since the build that last visited it, and that
//! shift accumulates down nested containers. A box under a `transform` or a
//! moving animation cannot be placed by layout rectangles alone, so no clip is
//! known below one until the next clipping ancestor. A margin of overscan keeps
//! the rows just outside the viewport real, so ordinary scrolling never shows
//! a placeholder; a wrong guess costs one frame, after which the placeholder's
//! own measured rectangle lowers the subtree again.
//!
//! `VN_NO_CULL=1` turns it off.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Styled as _, Window,
};

use crate::idmap::IdMap;
use crate::render::Inherited;
use crate::tree::{Kind, Node, Tree};
use crate::windows::{self, Rect, WinKey};

/// Extra space around a clip, as a fraction of its larger side, inside which
/// boxes are still lowered for real.
const OVERSCAN_FRACTION: f32 = 0.5;
/// The least overscan, in CSS pixels.
const MIN_OVERSCAN: f32 = 256.0;

/// One candidate's pin.
struct Entry {
    sub_rev: u64,
    ctx: Inherited,
    parent_size: (f32, f32),
    size: (f32, f32),
    /// The subtree's node ids, valid for `ids_rev`.
    ids: Vec<u32>,
    ids_rev: u64,
}

/// The clip a node's children are tested against, in this frame's window
/// coordinates, and how far last frame's rectangles of those children moved.
#[derive(Clone, Copy)]
struct View {
    /// `[left, top, right, bottom]`; `None` when no clip can be predicted.
    clip: Option<[f32; 4]>,
    shift: (f32, f32),
}

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static WINDOW: Cell<[f32; 4]> = const { Cell::new([0.0; 4]) };
    static ENTRIES: RefCell<IdMap<Entry>> = RefCell::new(IdMap::default());
    /// Per parent, for the build in progress.
    static VIEWS: RefCell<IdMap<View>> = RefCell::new(IdMap::default());
    /// Scroll offsets as of the last build that tested a container's children.
    static OFFSETS: RefCell<IdMap<(f32, f32)>> = RefCell::new(IdMap::default());
    /// Moving nodes and their ancestors: refused as candidates.
    static HOT: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    /// Moving nodes only: no clip is known below them.
    static MOVING: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    /// The focused and pointer-capturing nodes and their ancestors.
    static PINNED: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    /// Whether a subtree holds something that refuses culling, by `sub_rev`.
    static BLOCKED: RefCell<IdMap<(u64, bool)>> = RefCell::new(IdMap::default());
}

fn enabled() -> bool {
    thread_local! {
        static ON: Cell<i8> = const { Cell::new(-1) };
    }
    ON.with(|c| {
        if c.get() < 0 {
            let off = std::env::var("VN_NO_CULL")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    !(v.is_empty() || v == "0" || v == "false" || v == "off")
                })
                .unwrap_or(false);
            c.set(i8::from(!off));
        }
        c.get() == 1
    })
}

/// Decided once per build, before the tree is read.
pub(crate) fn begin_frame(window: &Window, now: std::time::Instant) {
    let on = enabled() && !window.is_a11y_active();
    ACTIVE.with(|c| c.set(on));
    VIEWS.with(|v| v.borrow_mut().clear());
    if !on {
        return;
    }
    let size = window.viewport_size();
    WINDOW.with(|c| c.set([0.0, 0.0, f32::from(size.width), f32::from(size.height)]));
    HOT.with(|hot| {
        MOVING.with(|moving| {
            let (mut hot, mut moving) = (hot.borrow_mut(), moving.borrow_mut());
            hot.clear();
            moving.clear();
            crate::tree::moving_paths(now, &mut hot);
            crate::tree::moving_nodes(now, &mut moving);
        })
    });
    let k = windows::key(window);
    let held = windows::peek(k, |w| [w.focused_node, w.pointer_capture.unwrap_or(0)])
        .unwrap_or([0, 0]);
    PINNED.with(|pinned| {
        let mut pinned = pinned.borrow_mut();
        pinned.clear();
        crate::tree::read_tree(|t| {
            for id in held {
                let mut cur = id;
                while cur != 0 && pinned.insert(cur) {
                    cur = t.node(cur).map_or(0, |n| n.parent);
                }
            }
        });
    });
}


/// Drops state for nodes that no longer exist — `render::gc_node_state`.
pub(crate) fn gc(live: &dyn Fn(u32) -> bool) {
    ENTRIES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    OFFSETS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    BLOCKED.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

fn clips(overflow: Option<gpui::Overflow>) -> bool {
    !matches!(overflow, None | Some(gpui::Overflow::Visible))
}

/// Does this ONE node refuse culling of any subtree holding it?
fn node_blocks(t: &Tree, id: u32, node: &Node) -> bool {
    if node.kind != Kind::Element {
        return false;
    }
    if crate::render::popped_out(id) || t.is_group_anchor(id) {
        return true;
    }
    if matches!(
        t.name(node.tag),
        "input" | "textarea" | "video" | "canvas" | "iframe"
    ) {
        return true;
    }
    let Some(cs) = &node.computed else {
        return false;
    };
    let base = &cs.base;
    if base.position == Some(gpui::Position::Absolute) || cs.extras.may_defer() {
        return true;
    }
    let inset = &base.inset;
    base.position == Some(gpui::Position::Relative)
        && (inset.top.is_some()
            || inset.right.is_some()
            || inset.bottom.is_some()
            || inset.left.is_some())
}

fn blocked(t: &Tree, id: u32) -> bool {
    let Some(node) = t.node(id) else {
        return false;
    };
    let rev = node.sub_rev;
    if let Some((at, hit)) = BLOCKED.with(|m| m.borrow().get(&id).copied()) {
        if at == rev {
            return hit;
        }
    }
    let out = node_blocks(t, id, node) || node.children.iter().any(|&c| blocked(t, c));
    BLOCKED.with(|m| m.borrow_mut().insert(id, (rev, out)));
    out
}

/// The clip `parent`'s children are tested against. The walk stops at the
/// build's root: a pop-out's DOM ancestors belong to another window.
fn view_of(t: &Tree, parent: u32, root: u32, win: WinKey) -> View {
    if let Some(view) = VIEWS.with(|v| v.borrow().get(&parent).copied()) {
        return view;
    }
    let window_view = View { clip: Some(WINDOW.with(Cell::get)), shift: (0.0, 0.0) };
    let view = match t.node(parent) {
        None => window_view,
        Some(node) => {
            let outer = if parent == root { window_view } else { view_of(t, node.parent, root, win) };
            let mut clip = outer.clip;
            let cs = node.computed.as_ref();
            if MOVING.with(|m| m.borrow().contains(&parent))
                || cs.is_some_and(|cs| cs.base.vn_transform.is_some())
            {
                clip = None;
            }
            let (clip_x, clip_y) = cs.map_or((false, false), |cs| {
                (clips(cs.base.overflow.x), clips(cs.base.overflow.y))
            });
            if clip_x || clip_y {
                // Its own box, where its ancestors' scrolling moved it.
                let own = windows::peek(win, |w| w.prev_bounds.get(&parent).copied()).flatten();
                if let Some((x, y, w, h)) = own {
                    let (x, y) = (x + outer.shift.0, y + outer.shift.1);
                    let mut c = clip.unwrap_or([f32::NEG_INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::INFINITY]);
                    if clip_x {
                        c[0] = c[0].max(x);
                        c[2] = c[2].min(x + w);
                    }
                    if clip_y {
                        c[1] = c[1].max(y);
                        c[3] = c[3].min(y + h);
                    }
                    clip = Some(c);
                }
            }
            let mut shift = outer.shift;
            if let Some(handle) = crate::render::scroll_handle_of(parent) {
                let offset = handle.offset();
                let now = (f32::from(offset.x), f32::from(offset.y));
                let before = OFFSETS.with(|m| m.borrow_mut().insert(parent, now)).unwrap_or(now);
                shift.0 += now.0 - before.0;
                shift.1 += now.1 - before.1;
            }
            View { clip, shift }
        }
    };
    VIEWS.with(|v| v.borrow_mut().insert(parent, view));
    view
}

/// The culling hook, called by `build_node` before it lowers anything: `Some`
/// is the placeholder the caller returns in place of the node.
pub(crate) fn placeholder(
    t: &Tree,
    id: u32,
    node: &Node,
    root: u32,
    ctx: impl FnOnce() -> Inherited,
    win: WinKey,
) -> Option<AnyElement> {
    if !ACTIVE.with(Cell::get) || id == root || node.parent == 0 || node.kind != Kind::Element {
        return None;
    }
    let cs = node.computed.as_ref()?;
    // Cheapest first: a box on screen costs one memo hit and one lookup.
    let view = view_of(t, node.parent, root, win);
    let clip = view.clip?;
    let (rect, parent) = windows::peek(win, |w| {
        (w.prev_bounds.get(&id).copied(), w.prev_bounds.get(&node.parent).copied())
    })?;
    let (x, y, width, height) = rect?;
    let (_, _, parent_width, parent_height) = parent?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let overscan = (clip[2] - clip[0]).max(clip[3] - clip[1]).min(1.0e6) * OVERSCAN_FRACTION;
    let overscan = overscan.max(MIN_OVERSCAN);
    let (x, y) = (x + view.shift.0, y + view.shift.1);
    let outside = x + width < clip[0] - overscan
        || x > clip[2] + overscan
        || y + height < clip[1] - overscan
        || y > clip[3] + overscan;
    if !outside {
        return None;
    }
    // Off screen: is the box a faithful stand-in?
    let independent = matches!(cs.base.display, Some(gpui::Display::Flex | gpui::Display::Grid))
        || clips(cs.base.overflow.x)
        || clips(cs.base.overflow.y)
        || t.node(node.parent)
            .and_then(|p| p.computed.as_ref())
            .is_some_and(|p| matches!(p.base.display, Some(gpui::Display::Flex | gpui::Display::Grid)));
    if !independent
        || HOT.with(|h| h.borrow().contains(&id))
        || PINNED.with(|p| p.borrow().contains(&id))
        || blocked(t, id)
    {
        return None;
    }
    let size = (width, height);
    let parent_size = (parent_width, parent_height);
    let sub_rev = node.sub_rev;
    let ctx = ctx();
    let fresh = ENTRIES.with(|m| {
        let mut m = m.borrow_mut();
        match m.get_mut(&id) {
            Some(e) if e.sub_rev == sub_rev && e.ctx == ctx && e.parent_size == parent_size && e.size == size => {
                if e.ids_rev != sub_rev || e.ids.is_empty() {
                    e.ids.clear();
                    crate::cache::subtree_ids(t, id, &mut e.ids);
                    e.ids_rev = sub_rev;
                }
                true
            }
            Some(e) => {
                e.sub_rev = sub_rev;
                e.ctx = ctx;
                e.parent_size = parent_size;
                e.size = size;
                false
            }
            None => {
                m.insert(id, Entry { sub_rev, ctx, parent_size, size, ids: Vec::new(), ids_rev: 0 });
                false
            }
        }
    });
    if !fresh {
        return None;
    }
    let mut boxed = gpui::div();
    *boxed.style() = crate::cache::wrapper_style(node, size);
    Some(Placeholder { id, inner: crate::render::probe(id, boxed.into_any_element()) }.into_any_element())
}

/// The stand-in: its probe records the box's rectangle, then every node under
/// it is carried forward by however far the box moved.
struct Placeholder {
    id: u32,
    inner: AnyElement,
}

impl IntoElement for Placeholder {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Placeholder {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.inner.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.prepaint(window, cx);
        let id = self.id;
        windows::here(window, |w| {
            let (Some(now), Some(before)) = (w.bounds.get(&id).copied(), w.prev_bounds.get(&id).copied())
            else {
                return;
            };
            let (dx, dy) = (now.0 - before.0, now.1 - before.1);
            let shifted = |r: Rect| (r.0 + dx, r.1 + dy, r.2, r.3);
            ENTRIES.with(|m| {
                let m = m.borrow();
                let Some(entry) = m.get(&id) else { return };
                for &node in entry.ids.iter().filter(|&&node| node != id) {
                    if let Some(r) = w.prev_bounds.get(&node).copied() {
                        w.bounds.insert(node, shifted(r));
                    }
                    if let Some(r) = w.prev_transformed.get(&node).copied() {
                        w.transformed.insert(node, shifted(r));
                    }
                }
            });
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.paint(window, cx);
    }
}
