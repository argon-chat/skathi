//! Retained-render cache boundaries — `Entity::cached()` over stable subtrees.
//!
//! gpui rebuilds and re-lays-out its whole element tree on every drawn frame by
//! design. On a small tree that is free (m11: 218 nodes, 3.9 ms of gpui);
//! measured at 1 000 `v-for` rows it is **not** — 5 011 nodes cost ~20 µs each,
//! 96 ms a frame, of which our own lowering is 25 ms and gpui's layout+paint
//! 67 ms (research/reports/m23/perf-retained.md §2). Nothing in `render.rs` is
//! doing anything wrong at 5 µs a node; there are simply too many nodes, every
//! drawn frame, however little of the page changed.
//!
//! gpui's own answer is `Entity<T>::cached(style)`: a view whose recorded
//! prepaint and paint spans are **replayed** — hitboxes, dispatch subtree, tab
//! stops, deferred draws, scene primitives and text layouts all copied forward
//! — while `render` is never called. It costs one thing: a cached view cannot
//! size to its content, so the composer has to supply the box.
//!
//! ## What decides a boundary
//!
//! * **Size.** We have the box: [`crate::render`]'s `BoundsProbe` records every
//!   painted node's rectangle every frame, so a subtree that did not change is
//!   pinned at the size it just laid out to. The pin is only sound while
//!   nothing can have resized it, which is what the rest of the freshness key
//!   is for — and the pin measures ITSELF once a boundary replays, which is why
//!   the box has to be confirmed by an ordinary build before it is trusted (see
//!   [`Entry::size`]).
//! * **Dirtiness.** [`Tree::touch`] bumps `sub_rev` on a node and every
//!   ancestor for every mutation, and the restyle pass touches every node whose
//!   fold actually moved — which is exactly the reach a descendant combinator,
//!   an inherited `var()` or a structural pseudo has. A boundary replays only
//!   while the number it rendered at still stands.
//! * **Reflow from outside.** A clean subtree can still need a different box
//!   when its *parent's* content box changes (a window resize, a sibling that
//!   grew). The parent's rectangle is in the same side map, so the boundary
//!   stores it and compares — one lookup per boundary per frame.
//! * **Everything gpui already knows.** `:hover`, `group-hover`, `:active`,
//!   wheel scrolling and focus all end in `cx.notify(current_view)` or
//!   `window.refresh()` inside gpui, and `Window::mark_view_dirty` walks the
//!   view path, so a notify on a nested cached view invalidates it and every
//!   cached ancestor. None of that is our problem, and none of it is bypassed.
//!
//! ## What is deliberately NOT cached (v1)
//!
//! A boundary is refused when its subtree contains anything whose per-frame
//! work does not survive a replay:
//!
//! * a **scroll container** — `vn_elements::scrollbar`'s wheel-routing registry
//!   is rebuilt per frame from the paint walk, and a replayed subtree does not
//!   walk, so the wheel would stop chaining. (Measured: caching a 5 000-node
//!   scroll container takes the mutation frame from 96 ms to **6.04 ms**, so
//!   this is the one exclusion worth removing next — it needs the registry to
//!   become replayable first.)
//! * **`<input>`/`<textarea>`, `<img>`, `<svg>`, `<video>`, `<lottie>`** —
//!   native widgets and decoders that repaint on their own clock;
//! * an **out-of-flow box** (`position: absolute|fixed`) or a **group anchor**
//!   (`.group(name)` is registered during the paint walk);
//! * anything at all, while a transition/animation, a Lottie or a video is
//!   running, or while a screen reader is attached — the AccessKit tree is
//!   built during the frame and is not part of the replayed spans.
//!
//! `VN_NO_CACHE=1` turns the whole thing off; `VN_CACHE_MIN=<n>` moves the
//! smallest subtree worth a boundary (default [`MIN_NODES`]). A boundary costs
//! about what one node costs — measured at ~18 µs against ~20 µs — so the win
//! on a subtree is its node count, and below a handful it is not worth having.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gpui::{AnyElement, AppContext as _, Context, Entity, IntoElement, Window};

use crate::render::{self, Inherited};
use crate::tree::{Kind, Node, Tree};
use crate::windows::{self, Rect, WinKey};

/// The smallest subtree that earns a cache boundary, in lowered nodes.
///
/// A boundary is one `ViewElement`, one element-state slot, one taffy leaf and
/// one replay span; measured on the 1 000-row scenario it costs ~18 µs against
/// the ~20 µs a lowered node costs, so a subtree of `n` nodes buys roughly `n`
/// back. Four is where that stops being noise.
const MIN_NODES: u32 = 4;

/// One node's cache boundary, and everything needed to decide whether it may
/// replay this frame.
struct Entry {
    /// Minted on the first offer, not on the first sighting: a subtree that
    /// never becomes stable enough to cache never costs an entity.
    entity: Option<Entity<CachedSubtree>>,
    /// [`Tree::sub_rev`] at the last render — "nothing inside changed".
    sub_rev: u64,
    /// The inherited context the subtree was lowered with. `text-transform`,
    /// `white-space`, `text-align`, `window-drag` and the stacking context are
    /// threaded down the build rather than folded, so a change to any of them
    /// at an ancestor changes the subtree's pixels without touching a node
    /// inside it.
    ctx: Inherited,
    /// The parent's rectangle at the last render — "nothing outside resized me".
    parent_rect: Option<Rect>,
    /// The box this boundary would be pinned at, as of the last frame.
    ///
    /// It is part of the freshness key, and that is the load-bearing half of the
    /// design: while a boundary replays, its own probe records the box we PINNED
    /// rather than the box its content would take, so the pin cannot correct
    /// itself. A boundary is therefore only offered once a frame that lowered
    /// the subtree for real has measured the same box twice running.
    size: (f32, f32),
    /// Was a boundary actually offered last frame? A subtree that fell back to
    /// an ordinary build has no recording gpui can trust, so the first offer
    /// after one must force a render.
    offered: bool,
    /// Every node id in the boundary's subtree — **not** only the ones this
    /// boundary lowered itself. Carried into the live bounds map on a replay
    /// frame, because the probes that would have recorded them do not run (see
    /// [`carry_bounds`]).
    ///
    /// The distinction is the whole of a bug worth remembering: boundaries
    /// NEST, and when an outer one replays, the inner ones' `boundary()` calls
    /// never run — they are inside the render that did not happen — so nothing
    /// else is ever going to carry their rectangles. `apps/hello` reported
    /// `bounds: null` for every tab in its nav the moment a Shift+Tab made the
    /// outer boundary replay.
    ids: Vec<u32>,
    /// gpui rendered this boundary since the last time it was offered.
    rendered: bool,
    /// Consecutive frames in which a boundary we believed CLEAN was re-rendered
    /// anyway — see [`THRASH_LIMIT`].
    misses: u8,
    /// Offers to skip before trying again, after a thrash.
    cooldown: u16,
}

/// Consecutive unasked-for re-renders before a boundary stands down.
///
/// A boundary earns its keep by replaying. gpui's cache key is `(bounds,
/// content mask, text style)`, so a subtree whose rectangle moves every frame —
/// every row of a list while it is being scrolled — re-renders every frame AND
/// pays the wrapper, the element state and the extra taffy leaf on top.
/// Measured on the 1 000-row scroll: 107 ms a frame without boundaries, 140 ms
/// with them. So a boundary that is asked to replay and does not, three times
/// running, stops being offered for [`COOLDOWN`] frames and the subtree lowers
/// the ordinary way.
///
/// Three, not one: the frame after a real mutation legitimately re-renders, and
/// so does the first frame after a scroll settles.
const THRASH_LIMIT: u8 = 3;

/// How many offers a stood-down boundary skips before trying again. At 165 Hz
/// this is ~0.15 s, so a scroll that has stopped starts replaying again within
/// a blink, and a scroll that has not pays the boundary overhead on one frame
/// in nine.
const COOLDOWN: u16 = 24;

thread_local! {
    static ENTRIES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
    /// Is caching allowed at all in the frame being built? Answered once per
    /// build by [`begin_frame`].
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The boundary currently being rendered — the recursion guard that stops
    /// `build_node` from wrapping the same node again on the way in.
    static IN_RENDER: Cell<u32> = const { Cell::new(0) };
    /// Memoised subtree shape, keyed by node id and validated against
    /// `sub_rev`: recomputing it is a walk of the subtree, which is the cost
    /// the boundary exists to avoid.
    static STATS: RefCell<HashMap<u32, (u64, Stats)>> = RefCell::new(HashMap::new());
}

/// `VN_NO_CACHE=1` — the A/B switch every "before" column in the report was
/// measured with.
fn enabled() -> bool {
    thread_local! {
        static ON: Cell<i8> = const { Cell::new(-1) };
    }
    ON.with(|c| {
        if c.get() < 0 {
            let off = std::env::var("VN_NO_CACHE")
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

fn min_nodes() -> u32 {
    thread_local! {
        static N: Cell<i64> = const { Cell::new(-1) };
    }
    N.with(|c| {
        if c.get() < 0 {
            c.set(
                std::env::var("VN_CACHE_MIN")
                    .ok()
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(MIN_NODES) as i64,
            );
        }
        c.get() as u32
    })
}

/// Decided once per build: may anything replay in this frame?
///
/// The four global refusals are the ones no per-node predicate can see. A
/// screen reader is the important one — the AccessKit tree is assembled during
/// the frame and is not part of the prepaint/paint spans gpui replays, so a
/// cached subtree would simply not exist for Narrator. Everything animating is
/// re-sampled from one `Instant` per build, so a replayed subtree would freeze
/// mid-transition.
pub(crate) fn begin_frame(window: &Window) {
    let on = enabled()
        && !window.is_a11y_active()
        && !crate::animating()
        && !crate::lotties_animating()
        && !crate::videos_playing();
    ACTIVE.with(|c| c.set(on));
}

/// The boundary whose subtree is being lowered right now, or `0`.
///
/// `build_node` asks so it can give that ONE node the same treatment it gives a
/// build root: a cached view is laid out as a root inside the box its wrapper
/// reserved, and a root with `width: auto` shrinks to fit. Without this a
/// stretched row came back 323 px wide instead of 1 518 — a visible change, and
/// a self-feeding one: the next frame pinned the wrong width, every row below
/// moved, and every boundary re-rendered forever.
#[inline]
pub(crate) fn boundary_root() -> u32 {
    IN_RENDER.with(|c| c.get())
}

/// May anything replay in the frame being built? One `Cell` read — the per-node
/// gate `build_node` gets before it assembles anything for [`boundary`].
#[inline]
pub(crate) fn active() -> bool {
    ACTIVE.with(|c| c.get())
}

/// Drops per-node state for nodes that no longer exist — called from
/// `render::gc_node_state`, off a JS wave.
pub(crate) fn gc(live: &dyn Fn(u32) -> bool) {
    ENTRIES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
    STATS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

/// What a subtree contains, for the "may this be a boundary" question.
#[derive(Clone, Copy, Default)]
struct Stats {
    /// Nodes that [`render::build_node`] would lower, this one included.
    nodes: u32,
    /// Something inside does per-frame work a replay would skip.
    blocked: bool,
}

/// Does this ONE node block a boundary over it (its children aside)?
fn node_blocks(t: &Tree, id: u32, node: &Node) -> bool {
    if node.kind != Kind::Element {
        return false;
    }
    // A pop-out paints this node from another window's build; the two must not
    // share a recording.
    if render::popped_out(id) {
        return true;
    }
    // `.group(name)` is registered on gpui's group stack during the PAINT walk,
    // and a replayed subtree does not walk.
    if t.is_group_anchor(id) {
        return true;
    }
    if matches!(
        t.name(node.tag),
        "input" | "textarea" | "img" | "svg" | "video" | "lottie" | "canvas" | "iframe"
    ) {
        return true;
    }
    let Some(cs) = &node.computed else {
        return false;
    };
    // A scroll container: `vn_elements::scrollbar`'s wheel-routing registry is
    // per-frame and rebuilt from the paint walk (see the module docs).
    if cs.base.overflow.x == Some(gpui::Overflow::Scroll)
        || cs.base.overflow.y == Some(gpui::Overflow::Scroll)
    {
        return true;
    }
    // Out of flow, or escaping its siblings' paint order: `render::stack` moves
    // the paint into gpui's deferred layer and an absolutely positioned box is
    // placed by its containing block rather than by its parent's flow, so
    // neither the pinned size nor the parent-rectangle check means what it says.
    if cs.base.position == Some(gpui::Position::Absolute)
        || cs.extras.deferred_priority().is_some()
    {
        return true;
    }
    // `position: relative` with an inset SHIFTS the box: the wrapper would have
    // to carry the shift and the inner element would then apply it again.
    cs.base.position == Some(gpui::Position::Relative) && has_inset(cs)
}

fn has_inset(cs: &vn_css::ComputedStates) -> bool {
    let i = &cs.base.inset;
    i.top.is_some() || i.right.is_some() || i.bottom.is_some() || i.left.is_some()
}

/// [`Stats`] for a subtree, memoised against its `sub_rev`.
fn stats(t: &Tree, id: u32) -> Stats {
    let Some(node) = t.node(id) else {
        return Stats::default();
    };
    let rev = node.sub_rev;
    if let Some(hit) = STATS.with(|m| m.borrow().get(&id).copied()) {
        if hit.0 == rev {
            return hit.1;
        }
    }
    let mut out = Stats { nodes: 1, blocked: node_blocks(t, id, node) };
    for &c in &node.children {
        let s = stats(t, c);
        out.nodes = out.nodes.saturating_add(s.nodes);
        out.blocked |= s.blocked;
    }
    STATS.with(|m| m.borrow_mut().insert(id, (rev, out)));
    out
}

/// The OUTER box a cached subtree presents to its parent.
///
/// A cached view is laid out from this style alone — `request_layout` never
/// sees the children — so everything that decides the node's used size has to
/// be here, and nothing that decides its INNER layout (padding, border) may be,
/// or it would be counted twice. The size is the one the node just laid out to,
/// which is what the two revision checks make safe to reuse.
fn wrapper_style(node: &Node, size: (f32, f32)) -> gpui::StyleRefinement {
    let mut st = gpui::StyleRefinement::default();
    st.size.width = Some(gpui::Length::Definite(gpui::px(size.0).into()));
    st.size.height = Some(gpui::Length::Definite(gpui::px(size.1).into()));
    // Pinned means pinned: the parent's flex must not stretch or shrink the box
    // away from the size the subtree was recorded at.
    st.flex_grow = Some(0.0);
    st.flex_shrink = Some(0.0);
    if let Some(cs) = &node.computed {
        // Margin is the node's spacing in its PARENT's layout, so it belongs
        // out here; the inner element keeps its own copy and taffy applies it
        // to a root node too, which is harmless because both agree.
        st.margin = cs.base.margin.clone();
        st.align_self = cs.base.align_self;
    }
    st
}

/// The view a boundary hands to gpui. It owns nothing but the coordinates of
/// the subtree it lowers; the tree itself stays where it is.
pub(crate) struct CachedSubtree {
    id: u32,
    root: u32,
    ctx: Inherited,
}

impl gpui::Render for CachedSubtree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (id, root, ctx) = (self.id, self.root, self.ctx);
        let prev = IN_RENDER.replace(id);
        let el = render::build_cached(id, root, ctx, window, cx);
        IN_RENDER.set(prev);
        el
    }
}

/// The boundary hook, called by `build_node` before it lowers anything.
///
/// `Some` means "this node is a cache boundary and here is its element"; the
/// caller must return it untouched — no probe, no stacking wrapper, because the
/// recording already contains both.
pub(crate) fn boundary<V: 'static>(
    t: &Tree,
    id: u32,
    node: &Node,
    root: u32,
    ctx: Inherited,
    win: WinKey,
    cx: &mut Context<V>,
) -> Option<AnyElement> {
    if id == root || IN_RENDER.with(|c| c.get()) == id {
        return None;
    }
    if node.kind != Kind::Element || node.computed.is_none() {
        return None;
    }
    let shape = stats(t, id);
    if shape.blocked || shape.nodes < min_nodes() {
        return None;
    }
    // The box the subtree laid out to last frame. Nothing to pin on the first
    // frame a node is painted, so the first frame is always a real build.
    let size = render::prev_size_of(id)?;

    let parent_rect = (node.parent != 0).then(|| render::prev_rect_of(node.parent)).flatten();

    let sub_rev = node.sub_rev;
    // Everything that has to hold for the recording to still describe this node.
    // `size` is in there for the reason spelled out on [`Entry::size`]; the rest
    // is "nothing inside changed" (`sub_rev`), "nothing an ancestor threads down
    // changed" (`ctx`) and "nothing outside resized me" (`parent_rect`).
    let offer = ENTRIES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m.entry(id).or_insert_with(|| Entry {
            entity: None,
            sub_rev,
            ctx,
            parent_rect,
            size,
            ids: Vec::new(),
            rendered: false,
            misses: 0,
            cooldown: 0,
            offered: false,
        });
        // A re-render nobody asked for: the boundary was offered as clean and
        // gpui went through it anyway, which means its rectangle, its content
        // mask or the text style around it moved. Three in a row and it is not
        // a cache — see [`THRASH_LIMIT`].
        if std::mem::take(&mut e.rendered) {
            e.misses = e.misses.saturating_add(1);
        } else {
            e.misses = 0;
        }
        if e.misses >= THRASH_LIMIT {
            e.misses = 0;
            e.cooldown = COOLDOWN;
        }
        if e.cooldown > 0 {
            e.cooldown -= 1;
            e.offered = false;
            crate::perf::note_cache_offer(true);
            return None;
        }
        let fresh =
            e.sub_rev == sub_rev && e.ctx == ctx && e.parent_rect == parent_rect && e.size == size;
        if !fresh {
            // Lower it the ordinary way this frame. That is what re-measures it
            // honestly — a wrapper would pin the very box we have just decided
            // we no longer believe — and it is cheaper than rendering through
            // the boundary only to throw the recording away.
            e.sub_rev = sub_rev;
            e.ctx = ctx;
            e.parent_rect = parent_rect;
            e.size = size;
            e.ids.clear();
            e.offered = false;
            return None;
        }
        // First offer after an ordinary build: gpui may still be holding an
        // element state describing a subtree we have since rebuilt, and it must
        // not be replayed.
        //
        // A FRESH ENTITY, not a `notify`. `notify` reaches `window.dirty_views`
        // through the effect queue, and this runs inside the draw it would have
        // to affect — so the invalidation lands a frame late and gpui replays
        // the stale recording in between. A new `EntityId` is a different
        // `ElementId`, so gpui has no state for it and renders, every time,
        // synchronously. (Observed as `apps/hello` reporting `bounds: null` for
        // every node under a boundary: a replay carries the rectangles the
        // recording holds, and that recording had none yet.)
        if !std::mem::replace(&mut e.offered, true) {
            e.entity = None;
            e.ids.clear();
        }
        let entity = e
            .entity
            .get_or_insert_with(|| cx.new(|_| CachedSubtree { id, root, ctx }))
            .clone();
        Some(entity)
    });
    let Some(entity) = offer else { return None };
    crate::perf::note_cache_offer(false);
    // Whether gpui replays or re-renders, the rectangles this subtree recorded
    // last frame are the rectangles it has now — the replay is only taken when
    // the bounds are identical, and a re-render overwrites them anyway.
    carry_bounds(id, win);
    Some(entity.cached(wrapper_style(node, size)).into_any_element())
}

/// Refills the live bounds map with the subtree's rectangles.
///
/// `getBoundingClientRect()`, the devtools highlight and every hit test read
/// that map, and `build_root` rotates it at the top of each build. A replayed
/// subtree runs no probes, so without this every node inside a cached boundary
/// would read back as "not painted" — the one compat invariant `Entity::cached`
/// breaks on its own, and the one it cannot see.
fn carry_bounds(id: u32, win: WinKey) {
    ENTRIES.with(|m| {
        let m = m.borrow();
        let Some(entry) = m.get(&id) else { return };
        if entry.ids.is_empty() {
            return;
        }
        windows::of(win, |w| {
            for id in &entry.ids {
                if let Some(r) = w.prev_bounds.get(id).copied() {
                    w.bounds.insert(*id, r);
                }
            }
        });
    });
}

/// Collects every node id in a subtree, in document order.
///
/// Walked once per boundary RENDER (never on a replay frame), which is the same
/// order as the render itself, and iterated flat on every replay.
pub(crate) fn subtree_ids(t: &Tree, id: u32, out: &mut Vec<u32>) {
    out.push(id);
    if let Some(node) = t.node(id) {
        for &c in &node.children {
            subtree_ids(t, c, out);
        }
    }
}

/// Records the ids one boundary's render covers — called by `build_cached`
/// once the subtree is built.
pub(crate) fn note_rendered(id: u32, ids: Vec<u32>) {
    ENTRIES.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(&id) {
            e.ids = ids;
            e.rendered = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{read_tree, restyle_all, set_stylesheet, with_tree};

    /// `(lowered nodes, blocked)` for a subtree, as the boundary predicate sees it.
    fn shape(id: u32) -> (u32, bool) {
        read_tree(|t| {
            let s = stats(t, id);
            (s.nodes, s.blocked)
        })
    }

    fn forget_shapes() {
        STATS.with(|m| m.borrow_mut().clear());
    }

    /// The node count is what decides whether a boundary is worth its ~18 µs,
    /// so it has to be the number of nodes the build would actually lower —
    /// text and anchors included, because each of those is a gpui element too.
    #[test]
    fn a_subtree_counts_every_node_the_build_would_lower() {
        let (row, _leaf) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let row = t.create_element(div);
            t.insert(row, root, 0);
            let mut last = 0;
            for _ in 0..4 {
                let cell = t.create_element(div);
                t.insert(cell, row, 0);
                last = cell;
            }
            (row, last)
        });
        forget_shapes();
        assert_eq!(shape(row), (5, false), "the row plus its four cells");
    }

    /// A scroll container anywhere inside refuses the boundary, however big the
    /// subtree is. `vn_elements::scrollbar`'s wheel-routing registry is rebuilt
    /// from the paint walk every frame, and a replayed subtree does not walk —
    /// so a cached scroll box would keep its pixels and lose its wheel.
    #[test]
    fn a_scroll_container_anywhere_inside_refuses_the_boundary() {
        set_stylesheet(".scrolls { overflow: scroll }".into());
        let (outer, plain) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let outer = t.create_element(div);
            let plain = t.create_element(div);
            let deep = t.create_element(div);
            let box_ = t.create_element(div);
            t.set_class(box_, "scrolls");
            t.insert(outer, root, 0);
            t.insert(plain, outer, 0);
            t.insert(deep, plain, 0);
            t.insert(box_, deep, 0);
            (outer, plain)
        });
        restyle_all();
        forget_shapes();
        assert!(shape(outer).1, "the scroll box is three levels down and still blocks");
        assert!(shape(plain).1, "and it blocks every ancestor of itself");
    }

    /// The shape is memoised against the subtree revision, because deriving it
    /// is the walk the boundary exists to avoid. A mutation has to be enough to
    /// re-derive it — otherwise a `<div>` that becomes a scroll container would
    /// keep a boundary that is no longer sound.
    #[test]
    fn the_shape_memo_follows_the_subtree_revision() {
        set_stylesheet(".scrolls { overflow: scroll }".into());
        let (wrap, inner) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let wrap = t.create_element(div);
            let inner = t.create_element(div);
            t.insert(wrap, root, 0);
            t.insert(inner, wrap, 0);
            (wrap, inner)
        });
        restyle_all();
        forget_shapes();
        assert_eq!(shape(wrap), (2, false), "nothing scrolls yet");

        with_tree(|t| t.set_class(inner, "scrolls"));
        restyle_all();
        assert!(shape(wrap).1, "the memo was invalidated by the mutation, not by a timer");
    }
}
