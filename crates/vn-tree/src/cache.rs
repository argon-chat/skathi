//! Retained-render cache boundaries — `Entity::cached()` over stable subtrees, whose recorded prepaint
//! and paint spans gpui replays without calling `render`.
//!
//! A cached view cannot size to its content, so the boundary pins the box the subtree last laid out
//! to. A boundary replays only while its freshness key stands: `sub_rev` (nothing inside changed),
//! the inherited context, and the parent's rectangle. Refused: native widgets and decoders that
//! repaint on their own clock, out-of-flow boxes, group anchors, subtrees holding a moving
//! animation, and everything while a screen reader is attached (the AccessKit tree is built during
//! the frame and is not part of the replayed spans).
//!
//! `VN_NO_CACHE=1` turns it off; `VN_CACHE_MIN=<n>` moves the smallest subtree worth a boundary.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Window,
};

use crate::idmap::IdMap;
use crate::render::{self, Inherited};
use crate::tree::{Kind, Node, Tree};
use crate::windows::{self, Rect, WinKey};

/// The smallest subtree that earns a cache boundary, in lowered nodes: a boundary costs about what
/// one lowered node does, so below this it is noise.
const MIN_NODES: u32 = 4;

/// One node's cache boundary, and everything needed to decide whether it may replay this frame.
struct Entry {
    /// Minted on the first offer, not on the first sighting: a subtree that never becomes stable
    /// enough to cache never costs an entity.
    entity: Option<Entity<CachedSubtree>>,
    /// [`Tree::sub_rev`] at the last render — "nothing inside changed".
    sub_rev: u64,
    /// `text-transform`, `white-space`, `text-align`, `window-drag` and the stacking context are
    /// threaded down the build rather than folded, so a change to any of them at an ancestor
    /// changes the subtree's pixels without touching a node inside it.
    ctx: Inherited,
    /// The parent's rectangle at the last render — "nothing outside resized me".
    parent_rect: Option<Rect>,
    /// The box this boundary would be pinned at, or `None` on the first sighting. Part of the
    /// freshness key: a replaying boundary's probe records the box it was PINNED to, so the pin cannot
    /// correct itself. A boundary is offered only once a real build measured the same box twice
    /// running; seeding it with the size in hand would pin a pre-stylesheet box forever.
    size: Option<(f32, f32)>,
    /// Last observed raw bounds.
    rect: Option<Rect>,
    /// A subtree that fell back to an ordinary build has no recording gpui can trust, so the first
    /// offer after one must force a render.
    offered: bool,
    /// Every node id in the boundary's subtree — not only the ones this boundary lowered itself.
    /// Boundaries nest, and when an outer one replays the inner `boundary()` calls never run, so
    /// nothing else would carry their rectangles into the bounds map (see [`carry_bounds`]).
    ids: Vec<u32>,
    // GPUI carries its own caption hitboxes on replay, but cannot refill these SDK-side facts.
    controls: Vec<(u32, gpui::WindowControlArea)>,
    /// The run of `vn_elements::scrollbar` wheel-registry entries the last real paint contributed,
    /// re-registered in place on replay (the registry is rebuilt from the paint walk, which a replay
    /// skips).
    chain: vn_elements::ChainSpan,
    /// gpui rendered this boundary since the last time it was offered.
    rendered: bool,
    /// Consecutive frames in which a boundary believed clean was re-rendered anyway ([`THRASH_LIMIT`]).
    misses: u8,
    /// Offers to skip before trying again, after a thrash.
    cooldown: u16,
}

/// Consecutive unasked-for re-renders before a boundary stands down. A subtree whose rectangle moves
/// every frame (a scrolling list) re-renders anyway and would pay the wrapper on top. Three, not
/// one: the frame after a real mutation legitimately re-renders.
const THRASH_LIMIT: u8 = 3;

/// How many offers a stood-down boundary skips before trying again.
const COOLDOWN: u16 = 24;

thread_local! {
    static ENTRIES: RefCell<IdMap<Entry>> = RefCell::new(IdMap::default());
    /// Is caching allowed at all in the frame being built?
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The boundary currently being rendered — the recursion guard that stops `build_node` from
    /// wrapping the same node again on the way in.
    static IN_RENDER: Cell<u32> = const { Cell::new(0) };
    /// Memoised subtree shape, keyed by node id and validated against `sub_rev`: recomputing it is
    /// a walk of the subtree, which is the cost the boundary exists to avoid.
    static STATS: RefCell<HashMap<u32, (u64, Stats)>> = RefCell::new(HashMap::new());
    /// The nodes the animator is moving in the frame being built, and every ancestor of each — the
    /// boundaries [`begin_frame`] refuses.
    static HOT: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
}

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

/// `VN_CACHE_NO_SCROLL=1` — put the scroll-container exclusion back.
fn refuse_scroll_containers() -> bool {
    thread_local! {
        static ON: Cell<i8> = const { Cell::new(-1) };
    }
    ON.with(|c| {
        if c.get() < 0 {
            let on = std::env::var("VN_CACHE_NO_SCROLL")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    !(v.is_empty() || v == "0" || v == "false" || v == "off")
                })
                .unwrap_or(false);
            c.set(i8::from(on));
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

/// Decided once per build: may anything replay in this frame, and which subtrees are moving? Only
/// the subtree holding a moving node (and its ancestors) is refused ([`HOT`]), since a replay would
/// freeze it mid-transition. The frame a node stops moving must not replay either: the first offer
/// after a refusal mints a fresh entity, which renders at `now`.
pub(crate) fn begin_frame(window: &Window, now: std::time::Instant) {
    let on = enabled() && !window.is_a11y_active() && !crate::resample::holds_replays();
    ACTIVE.with(|c| c.set(on));
    HOT.with(|h| {
        let mut h = h.borrow_mut();
        h.clear();
        if on {
            crate::tree::moving_paths(now, &mut h);
        }
    });
}

/// Is a node the build is sampling an animation for inside `id`'s subtree?
fn hot(id: u32) -> bool {
    HOT.with(|h| h.borrow().contains(&id))
}

/// The boundary whose subtree is being lowered right now, or `0`. `build_node` gives that node the
/// treatment of a build root: a cached view is laid out as a root in its wrapper's box, and a
/// `width: auto` root would shrink to fit and pin the wrong width.
#[inline]
pub(crate) fn boundary_root() -> u32 {
    IN_RENDER.with(|c| c.get())
}

/// May anything replay in the frame being built?
#[inline]
pub(crate) fn active() -> bool {
    ACTIVE.with(|c| c.get())
}

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
    // A pop-out paints this node from another window's build; the two must not share a recording.
    if render::popped_out(id) {
        return true;
    }
    // `.group(name)` is registered on gpui's group stack during the PAINT walk, and a replayed
    // subtree does not walk.
    if t.is_group_anchor(id) {
        return true;
    }
    let tag = t.name(node.tag);
    if (tag == "svg" && !crate::svg::can_replay(t, id)) || matches!(
        tag,
        "input" | "textarea" | "img" | "video" | "canvas" | "iframe"
    ) {
        return true;
    }
    let Some(cs) = &node.computed else {
        return false;
    };
    // A referenced definition can change outside this subtree, so its matrix must be resolved
    // afresh; descendants still reuse the matrix-aware cache.
    if cs.extras.filter_url.is_some() { return true; }
    // The wheel registry it contributes to is rebuilt from the paint walk and a replay does not
    // walk, so the span it pushed is recorded and re-registered in place instead — see
    // [`Boundary`].
    if refuse_scroll_containers()
        && (cs.base.overflow.x == Some(gpui::Overflow::Scroll)
            || cs.base.overflow.y == Some(gpui::Overflow::Scroll))
    {
        return true;
    }
    if cs.base.position == Some(gpui::Position::Absolute)
        || cs.extras.may_defer()
    {
        return true;
    }
    // `position: relative` with an inset SHIFTS the box: the wrapper would have to carry the shift
    // and the inner element would then apply it again.
    cs.base.position == Some(gpui::Position::Relative) && has_inset(cs)
}

fn has_inset(cs: &vn_css::ComputedStates) -> bool {
    let i = &cs.base.inset;
    i.top.is_some() || i.right.is_some() || i.bottom.is_some() || i.left.is_some()
}

fn block_flow(cs: &vn_css::ComputedStates) -> bool {
    matches!(cs.base.display, None | Some(gpui::Display::Block))
}

/// Can a child's vertical margin adjoin this box's `bottom` (or top) edge?
fn edge_open(cs: &vn_css::ComputedStates, bottom: bool) -> bool {
    use gpui::IsZero;
    let b = &cs.base;
    let visible = |o: Option<gpui::Overflow>| matches!(o, None | Some(gpui::Overflow::Visible));
    let (padding, border) = if bottom {
        (&b.padding.bottom, &b.border_widths.bottom)
    } else {
        (&b.padding.top, &b.border_widths.top)
    };
    block_flow(cs)
        && visible(b.overflow.x)
        && visible(b.overflow.y)
        && b.position != Some(gpui::Position::Absolute)
        && cs.extras.fixed != Some(true)
        && padding.as_ref().is_none_or(|p| p.is_zero())
        && border.as_ref().is_none_or(|w| w.is_zero())
        && (!bottom || matches!(b.size.height, None | Some(gpui::Length::Auto)))
}

/// Does a descendant's margin collapse THROUGH one of this node's vertical edges, so that it spaces
/// the node in its parent's flow? The wrapper reserves only the node's own margin, so such a node
/// would be a pixel off whenever it replays; erring towards `true` only costs a replay.
fn margin_leaks(t: &Tree, node: &Node) -> bool {
    let Some(cs) = &node.computed else {
        return false;
    };
    let parent = t.node(node.parent).and_then(|p| p.computed.as_ref());
    if !block_flow(cs) || parent.is_some_and(|p| !block_flow(p)) {
        return false;
    }
    [false, true].into_iter().any(|bottom| edge_leaks(t, node, bottom))
}

fn edge_leaks(t: &Tree, node: &Node, bottom: bool) -> bool {
    use gpui::IsZero;
    let mut at = node;
    for _ in 0..32 {
        if !at.computed.as_ref().is_some_and(|cs| edge_open(cs, bottom)) {
            return false;
        }
        // Anchors and collapsed whitespace make no box; `display: none` and out-of-flow boxes are
        // not in the flow.
        let in_flow = |&&c: &&u32| {
            t.node(c).is_some_and(|n| match n.kind {
                Kind::Anchor => false,
                Kind::Text => !n.text.trim().is_empty(),
                Kind::Element => n.computed.as_ref().is_none_or(|cs| {
                    cs.base.display != Some(gpui::Display::None)
                        && cs.base.position != Some(gpui::Position::Absolute)
                        && cs.extras.fixed != Some(true)
                }),
            })
        };
        let child = if bottom {
            at.children.iter().rev().find(in_flow)
        } else {
            at.children.iter().find(in_flow)
        };
        let Some(child) = child.and_then(|&c| t.node(c)) else {
            return false;
        };
        if child.kind != Kind::Element {
            return false;
        }
        let Some(cs) = &child.computed else {
            return false;
        };
        let margin = if bottom { &cs.base.margin.bottom } else { &cs.base.margin.top };
        if matches!(margin, Some(gpui::Length::Definite(m)) if !m.is_zero()) {
            return true;
        }
        at = child;
    }
    true
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
    // SVG serializes its children into one leaf; counting shape nodes would offer a cache wrapper
    // for a single native element with a large XML tree.
    if node.kind != Kind::Element || t.name(node.tag) != "svg" {
        for &c in &node.children {
            let s = stats(t, c);
            out.nodes = out.nodes.saturating_add(s.nodes);
            out.blocked |= s.blocked;
        }
    }
    STATS.with(|m| m.borrow_mut().insert(id, (rev, out)));
    out
}

/// The OUTER box a cached subtree presents to its parent. A cached view is laid out from this style
/// alone, so everything deciding the node's used size is here and nothing deciding its inner layout
/// (padding, border) may be.
pub(crate) fn wrapper_style(node: &Node, size: (f32, f32)) -> gpui::StyleRefinement {
    let mut st = gpui::StyleRefinement::default();
    st.size.width = Some(gpui::Length::Definite(gpui::px(size.0).into()));
    st.size.height = Some(gpui::Length::Definite(gpui::px(size.1).into()));
    st.flex_grow = Some(0.0);
    st.flex_shrink = Some(0.0);
    if let Some(cs) = &node.computed {
        // Margin is the node's spacing in its PARENT's layout, so it belongs only here.
        // normalize_inner_root removes its inner duplicate.
        st.margin = cs.base.margin.clone();
        st.align_self = cs.base.align_self;
    }
    st
}

/// The cached view already resolved its root's outer geometry; resolving percentage widths against
/// the pinned box a second time would make an 85% column 85% of itself.
pub(crate) fn normalize_inner_root(id: u32, style: &mut gpui::StyleRefinement) {
    if IN_RENDER.with(Cell::get) != id {
        return;
    }
    let size = ENTRIES.with(|entries| entries.borrow().get(&id).and_then(|entry| entry.size));
    if let Some(size) = size {
        pin_inner_style(style, size);
    }
}

fn pin_inner_style(style: &mut gpui::StyleRefinement, size: (f32, f32)) {
    style.size.width = Some(gpui::px(size.0).into());
    style.size.height = Some(gpui::px(size.1).into());
    style.min_size.width = Some(gpui::px(0.0).into());
    style.min_size.height = Some(gpui::px(0.0).into());
    style.max_size.width = Some(gpui::Length::Auto);
    style.max_size.height = Some(gpui::Length::Auto);
    style.flex_basis = Some(gpui::Length::Auto);
    style.flex_grow = Some(0.0);
    style.flex_shrink = Some(0.0);
    style.align_self = None;
    style.margin.top = Some(gpui::px(0.0).into());
    style.margin.right = Some(gpui::px(0.0).into());
    style.margin.bottom = Some(gpui::px(0.0).into());
    style.margin.left = Some(gpui::px(0.0).into());
}

/// The view a boundary hands to gpui; it owns only the coordinates of the subtree it lowers.
pub(crate) struct CachedSubtree {
    id: u32,
    root: u32,
    ctx: Inherited,
}

impl gpui::Render for CachedSubtree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (id, root, ctx) = (self.id, self.root, self.ctx.clone());
        let prev = IN_RENDER.replace(id);
        let el = render::build_cached(id, root, ctx, window, cx);
        IN_RENDER.set(prev);
        el
    }
}

/// The boundary hook, called by `build_node` before it lowers anything.
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
    if shape.blocked || shape.nodes < min_nodes() || margin_leaks(t, node) {
        return None;
    }
    if hot(id) {
        // The frame it stops moving must render, not replay (see [`begin_frame`]).
        with_entry(id, |e| e.offered = false);
        return None;
    }
    let rect = render::prev_rect_of(id).filter(|(_, _, width, height)| *width > 0.0 && *height > 0.0)?;
    let size = (rect.2, rect.3);

    let parent_rect = (node.parent != 0).then(|| render::prev_rect_of(node.parent)).flatten();

    let sub_rev = node.sub_rev;
    // `size` is in there for the reason spelled out on [`Entry::size`]; the rest is "nothing inside
    // changed" (`sub_rev`), "nothing an ancestor threads down changed" (`ctx`) and "nothing outside
    // resized me" (`parent_rect`).
    let offer = ENTRIES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m.entry(id).or_insert_with(|| Entry {
            entity: None,
            sub_rev,
            ctx: ctx.clone(),
            parent_rect,
            // NOT `Some(size)`: the first sighting must not compare equal to itself and offer on
            // one frame's measurement — see [`Entry::size`].
            size: None,
            rect: None,
            ids: Vec::new(),
            controls: Vec::new(),
            chain: Default::default(),
            rendered: false,
            misses: 0,
            cooldown: 0,
            offered: false,
        });
        // A re-render nobody asked for: the rectangle, content mask or text style around it moved.
        // Three in a row and it is not a cache ([`THRASH_LIMIT`]).
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
            e.sub_rev == sub_rev
                && e.ctx == ctx
                && e.parent_rect == parent_rect
                && e.size == Some(size)
                && e.rect == Some(rect);
        if !fresh {
            e.sub_rev = sub_rev;
            e.ctx = ctx.clone();
            e.parent_rect = parent_rect;
            e.size = Some(size);
            e.rect = Some(rect);
            e.ids.clear();
            e.controls.clear();
            e.chain = Default::default();
            e.offered = false;
            return None;
        }
        // First offer after an ordinary build: gpui may still hold an element state describing a
        // subtree we have since rebuilt. A FRESH ENTITY, not a `notify`: `notify` lands a frame late
        // (this runs inside the draw it would affect) and gpui would replay the stale recording; a
        // new `EntityId` has no state, so gpui renders synchronously.
        if !std::mem::replace(&mut e.offered, true) {
            e.entity = None;
            e.ids.clear();
            e.controls.clear();
            e.chain = Default::default();
        }
        let entity = e
            .entity
            .get_or_insert_with(|| cx.new(|_| CachedSubtree { id, root, ctx }))
            .clone();
        Some(entity)
    });
    let Some(entity) = offer else { return None };
    crate::perf::note_cache_offer(false);
    carry_bounds(id, win);
    Some(
        Boundary { id, inner: entity.cached(wrapper_style(node, size)).into_any_element() }
            .into_any_element(),
    )
}

/// The one thing a replay has to be given back by hand: the scroll chain. A real paint records the
/// run of wheel-registry entries the subtree pushed; a replay re-pushes it in place, since the
/// registry is read innermost-first and that order is positional.
struct Boundary {
    id: u32,
    inner: AnyElement,
}

impl Element for Boundary {
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
        let mark = vn_elements::scrollbar::chain_mark();
        self.inner.paint(window, cx);
        let win = windows::key(window);
        with_entry(self.id, |entry| {
            if entry.rendered {
                entry.controls = windows::peek(win, |state| {
                    state.control_hitboxes.iter()
                        .filter(|(id, _)| entry.ids.contains(id)).copied().collect()
                }).unwrap_or_default();
            }
        });
        let span = vn_elements::scrollbar::chain_since(mark);
        if span.is_empty() {
            // Nothing registered: either this subtree really has no scroll container (the span is
            // empty too, and this is a no-op), or gpui replayed the recording and the walk that
            // would have registered them never ran.
            with_entry(self.id, |e| vn_elements::scrollbar::chain_replay(&e.chain));
        } else {
            with_entry(self.id, |e| e.chain = span);
        }
    }
}

/// Runs `f` against one boundary's entry, if it still has one.
fn with_entry(id: u32, f: impl FnOnce(&mut Entry)) {
    ENTRIES.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(&id) {
            f(e);
        }
    });
}

/// Refills the live bounds map with the subtree's rectangles. A replayed subtree runs no probes, so
/// without this its nodes would read as unpainted (`getBoundingClientRect`, hit tests, devtools).
fn carry_bounds(id: u32, win: WinKey) {
    ENTRIES.with(|m| {
        let m = m.borrow();
        let Some(entry) = m.get(&id) else { return };
        if entry.ids.is_empty() {
            return;
        }
        windows::of(win, |w| {
            restore_controls(w, &entry.controls);
            for id in &entry.ids {
                if let Some(r) = w.prev_bounds.get(id).copied() {
                    w.bounds.insert(*id, r);
                }
                if let Some(r) = w.prev_transformed.get(id).copied() {
                    w.transformed.insert(*id, r);
                }
            }
        });
    });
}

fn restore_controls(state: &mut windows::WindowState, controls: &[(u32, gpui::WindowControlArea)]) {
    for &(id, area) in controls {
        state.any_drag_region |= area == gpui::WindowControlArea::Drag;
        if !state.control_hitboxes.contains(&(id, area)) {
            state.control_hitboxes.push((id, area));
        }
    }
}

/// Collects every node id in a subtree, in document order.
pub(crate) fn subtree_ids(t: &Tree, id: u32, out: &mut Vec<u32>) {
    out.push(id);
    if let Some(node) = t.node(id) {
        for &c in &node.children {
            subtree_ids(t, c, out);
        }
    }
}

/// Records the ids one boundary's render covers — called by `build_cached` once the subtree is
/// built. The recorded scene spans themselves live in gpui.
pub(crate) fn mem_stats() -> (usize, usize, usize) {
    ENTRIES.with(|m| {
        let m = m.borrow();
        (m.len(), m.values().filter(|e| e.entity.is_some()).count(), m.values().map(|e| e.ids.len()).sum())
    })
}

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

    #[test]
    fn cached_registry_keeps_caption_regions_after_frame_reset_and_nested_restore() {
        let win = 81231;
        let id = 81232;
        let controls = vec![(id, gpui::WindowControlArea::Drag), (id + 1, gpui::WindowControlArea::NoDrag)];
        ENTRIES.with(|entries| entries.borrow_mut().insert(id, Entry {
            entity: None, sub_rev: 0, ctx: Default::default(), parent_rect: None,
            size: Some((200., 40.)), rect: Some((0., 0., 200., 40.)), offered: true, ids: vec![id, id + 1], controls: controls.clone(),
            chain: Default::default(), rendered: false, misses: 0, cooldown: 0,
        }));
        for _ in 0..20 {
            windows::of(win, |state| { state.control_hitboxes.clear(); state.any_drag_region = false; });
            carry_bounds(id, win);
            carry_bounds(id, win);
            windows::of(win, |state| {
                assert_eq!(state.control_hitboxes, controls, "nested restoration must not duplicate caption areas");
                assert!(state.any_drag_region, "a replayed caption is still a painted drag region");
            });
        }
        ENTRIES.with(|entries| entries.borrow_mut().remove(&id));
        windows::forget(win);
    }

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

    /// A group of rows as a channel list has them: `group > rows > row`, the row carrying the
    /// margin.
    fn group_of_rows(css: &str) -> (u32, u32, u32) {
        set_stylesheet(css.into());
        let ids = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mut make = |t: &mut Tree, class: &str, parent: u32| {
                let el = t.create_element(div);
                t.set_class(el, class.into());
                t.insert(el, parent, 0);
                el
            };
            let list = make(t, "list", root);
            let group = make(t, "group", list);
            let rows = make(t, "rows", group);
            let _first = make(t, "row", rows);
            let last = make(t, "row", rows);
            (group, rows, last)
        });
        restyle_all();
        ids
    }

    fn leaks(id: u32) -> bool {
        read_tree(|t| margin_leaks(t, t.node(id).unwrap()))
    }

    #[test]
    fn a_box_a_childs_margin_collapses_through_is_not_a_boundary() {
        let (group, rows, last) = group_of_rows(".row { margin-bottom: 1px; height: 30px }");
        assert!(leaks(group), "the last row's margin spaces the GROUP in the list");
        assert!(leaks(rows));
        assert!(!leaks(last), "a row's own margin is in its wrapper");
    }

    #[test]
    fn a_closed_edge_keeps_the_margin_inside() {
        for closed in [
            ".rows { padding-bottom: 1px }",
            ".rows { border-bottom: 1px solid red }",
            ".rows { overflow: hidden }",
            ".rows { height: 80px }",
            ".rows { display: flex; flex-direction: column }",
        ] {
            let (group, rows, _) = group_of_rows(&format!(".row {{ margin-bottom: 1px; height: 30px }} {closed}"));
            assert!(!leaks(group), "{closed}");
            assert!(!leaks(rows), "{closed}");
        }
        let (group, _, _) = group_of_rows(".row { margin-bottom: 1px } .list { display: flex; flex-direction: column }");
        assert!(!leaks(group), "a flex item is its own formatting context");
        let (group, rows, _) = group_of_rows(".row { height: 30px }");
        assert!(!leaks(group) && !leaks(rows), "no margin, nothing to lose");
    }

    #[test]
    fn a_top_margin_leaks_like_a_bottom_one() {
        let (group, rows, _) = group_of_rows(".row { margin-top: 4px }");
        assert!(leaks(group) && leaks(rows));
        let (group, rows, _) = group_of_rows(".row { margin-top: 4px } .group { padding-top: 2px }");
        assert!(!leaks(group), "the group's padding stops it");
        assert!(leaks(rows));
    }


    fn mono_icon(t: &mut Tree, parent: u32) -> (u32, u32) {
        let svg_tag = t.intern("svg");
        let svg = t.create_element(svg_tag);
        for (name, value) in [("viewBox", "0 0 24 24"), ("fill", "none"), ("stroke", "currentColor")] {
            let key = t.intern(name);
            t.set_prop(svg, key, Some(value.into()));
        }
        t.insert(svg, parent, 0);
        let path_tag = t.intern("path");
        let path = t.create_element(path_tag);
        let geometry = t.intern("d");
        t.set_prop(path, geometry, Some("M4 12l5 5L20 6".into()));
        t.insert(path, svg, 0);
        (svg, path)
    }

    #[test]
    fn svg_fragment_cannot_hide_a_resource_dependency() {
        set_stylesheet(String::new());
        let svg = with_tree(|t| {
            let root = t.create_root();
            let (svg, path) = mono_icon(t, root);
            let fragment = t.create_anchor();
            t.insert(fragment, svg, 0);
            t.insert(path, fragment, 0);
            let href = t.intern("href");
            t.set_prop(path, href, Some("https://example.invalid/shape".into()));
            svg
        });
        restyle_all();
        forget_shapes();
        assert!(shape(svg).1, "transparent fragments must not hide dependencies from replay eligibility");
    }

    #[test]
    fn self_contained_mono_svg_counts_as_one_lowered_leaf() {
        set_stylesheet(String::new());
        let (row, svg) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let row = t.create_element(div);
            t.insert(row, root, 0);
            let (svg, _) = mono_icon(t, row);
            for _ in 0..10 {
                let path_tag = t.intern("path");
                let path = t.create_element(path_tag);
                t.insert(path, svg, 0);
            }
            for _ in 0..3 {
                let child = t.create_element(div);
                t.insert(child, row, 0);
            }
            (row, svg)
        });
        restyle_all();
        forget_shapes();
        assert_eq!(shape(svg), (1, false));
        assert_eq!(shape(row), (5, false), "SVG shape nodes are not native layout nodes");
    }

    #[test]
    fn colour_and_external_svg_still_block_ancestors() {
        set_stylesheet(String::new());
        let (row, svg, path) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let row = t.create_element(div);
            t.insert(row, root, 0);
            let (svg, path) = mono_icon(t, row);
            (row, svg, path)
        });
        restyle_all();
        forget_shapes();
        assert!(!shape(row).1);
        with_tree(|t| {
            let stroke = t.intern("stroke");
            t.set_prop(svg, stroke, Some("red".into()));
        });
        restyle_all();
        assert!(shape(row).1, "late concrete paint must invalidate the memoized eligibility");
        with_tree(|t| {
            let stroke = t.intern("stroke");
            t.set_prop(svg, stroke, Some("currentColor".into()));
            let href = t.intern("href");
            t.set_prop(path, href, Some("https://example.invalid/icon.svg#shape".into()));
        });
        restyle_all();
        assert!(shape(row).1, "an href blocks even when the classifier still says Mono");
        with_tree(|t| {
            let href = t.intern("href");
            t.set_prop(path, href, None);
        });
        restyle_all();
        assert!(!shape(row).1, "removing the resource dependency restores eligibility");
    }

    #[test]
    fn svg_stylesheet_paints_and_currentcolor_invalidate_ancestor_revisions() {
        set_stylesheet(".icon { color: red }".into());
        let (row, svg) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let row = t.create_element(div);
            t.insert(row, root, 0);
            let (svg, _) = mono_icon(t, row);
            t.set_class(svg, "icon");
            (row, svg)
        });
        restyle_all();
        forget_shapes();
        assert!(!shape(row).1);
        let before = read_tree(|t| (t.sub_rev(row), t.node(svg).unwrap().computed.as_ref().unwrap().base.text.color));
        set_stylesheet(".icon { color: blue }".into());
        with_tree(|t| t.mark_dirty());
        restyle_all();
        let after = read_tree(|t| (t.sub_rev(row), t.node(svg).unwrap().computed.as_ref().unwrap().base.text.color));
        assert_ne!(before.0, after.0, "a cached ancestor must rebuild its recorded tint");
        assert_ne!(before.1, after.1);
        assert!(!shape(row).1, "currentColor tint changes do not require a colour raster");
        set_stylesheet(".icon { color: blue } .icon path { stroke: red }".into());
        with_tree(|t| t.mark_dirty());
        restyle_all();
        assert!(shape(row).1, "a descendant's computed concrete stroke invalidates cached Stats");
    }

    #[test]
    fn svg_resource_and_progress_forms_stay_excluded() {
        for tag in ["use", "image", "text", "filter", "foreignObject", "animate", "animateTransform", "set", "svg"] {
            set_stylesheet(String::new());
            let svg = with_tree(|t| {
                let root = t.create_root();
                let (svg, _) = mono_icon(t, root);
                let child_tag = t.intern(tag);
                let child = t.create_element(child_tag);
                t.insert(child, svg, 0);
                svg
            });
            restyle_all();
            forget_shapes();
            assert!(shape(svg).1, "<{tag}> cannot enter static geometry replay");
        }
        for (name, value) in [("xlink:href", "#local"), ("begin", "1s"), ("dur", "2s"), ("clip-path", "url(#external)"), ("stroke", "context-stroke")] {
            set_stylesheet(String::new());
            let svg = with_tree(|t| {
                let root = t.create_root();
                let (svg, path) = mono_icon(t, root);
                let key = t.intern(name);
                t.set_prop(path, key, Some(value.into()));
                svg
            });
            restyle_all();
            forget_shapes();
            assert!(shape(svg).1, "{name}={value} must stay excluded");
        }
        set_stylesheet(".icon { filter: drop-shadow(1px 1px 2px red) }".into());
        let svg = with_tree(|t| {
            let root = t.create_root();
            let (svg, _) = mono_icon(t, root);
            t.set_class(svg, "icon");
            svg
        });
        restyle_all();
        forget_shapes();
        assert!(shape(svg).1, "drop-shadow selects the colour raster branch even for a Mono document");
    }

    #[test]
    fn svg_own_hover_and_transition_keep_the_normal_hot_exclusion() {
        set_stylesheet(".icon { color: red; transition: color 100ms } .icon:hover { color: blue }".into());
        let svg = with_tree(|t| {
            let root = t.create_root();
            let (svg, _) = mono_icon(t, root);
            t.set_class(svg, "icon");
            svg
        });
        restyle_all();
        forget_shapes();
        read_tree(|t| {
            let cs = t.node(svg).unwrap().computed.as_ref().unwrap();
            assert!(cs.hover.is_some());
            assert!(!cs.transitions.is_empty());
        });
        assert!(!shape(svg).1, "root states use native replayed listeners, not a raster clock");
        let before_hover = std::time::Instant::now();
        crate::tree::note_hover(svg, true);
        let mut moving = HashSet::new();
        crate::tree::moving_paths(before_hover, &mut moving);
        assert!(moving.contains(&svg), "the eligible root's colour transition must remain HOT");
        let parent = read_tree(|t| t.node(svg).unwrap().parent);
        assert!(moving.contains(&parent), "a boundary over the icon must also be HOT");
        moving.clear();
        crate::tree::moving_paths(std::time::Instant::now() + std::time::Duration::from_millis(200), &mut moving);
        assert!(!moving.contains(&svg), "finished transitions may become cacheable again");
        crate::tree::note_hover(svg, false);
    }

    /// The node count is what decides whether a boundary is worth its ~18 µs, so it has to be the
    /// number of nodes the build would actually lower — text and anchors included, because each of
    /// those is a gpui element too.
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

    #[test]
    fn cached_root_uses_resolved_border_box_without_reapplying_percent_constraints() {
        let mut diagnostics = Vec::new();
        let parsed = vn_css::parse_inline_style(
            "width: 85%; max-width: 85%; min-width: 50%; height: 50%; max-height: 75%; \
             padding: 12px; border: 2px solid red; margin: 7px 9px; \
             display: flex; flex-direction: column; align-items: flex-end; flex: 1 1 30%",
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let mut inner = (*parsed.refinement.unwrap()).clone();
        let padding = inner.padding.clone();
        let border = inner.border_widths.clone();
        let alignment = inner.align_items;
        pin_inner_style(&mut inner, (520.0, 148.0));
        assert_eq!(inner.size.width, Some(gpui::px(520.0).into()), "520px must not become 85% of 520px");
        assert_eq!(inner.size.height, Some(gpui::px(148.0).into()));
        assert_eq!(inner.max_size.width, Some(gpui::Length::Auto));
        assert_eq!(inner.max_size.height, Some(gpui::Length::Auto));
        assert_eq!(inner.min_size.width, Some(gpui::px(0.0).into()));
        assert_eq!(inner.min_size.height, Some(gpui::px(0.0).into()));
        assert_eq!(inner.padding, padding, "padding remains inside the recorded border box");
        assert_eq!(inner.border_widths, border);
        assert_eq!(inner.align_items, alignment, "right-aligned children keep their alignment");
        assert_eq!(inner.flex_direction, Some(gpui::FlexDirection::Column));
        assert_eq!(inner.flex_basis, Some(gpui::Length::Auto));
        assert_eq!(inner.margin.left, Some(gpui::px(0.0).into()), "the cache wrapper alone owns the margin");
        assert_eq!(inner.margin.top, Some(gpui::px(0.0).into()));
    }

    #[test]
    fn ordinary_nodes_keep_authored_percent_geometry() {
        let mut diagnostics = Vec::new();
        let parsed = vn_css::parse_inline_style("max-width: 85%; margin-left: auto; padding: 8px", &mut diagnostics);
        let mut style = (*parsed.refinement.unwrap()).clone();
        let authored = style.clone();
        let previous = IN_RENDER.replace(1234);
        normalize_inner_root(5678, &mut style);
        IN_RENDER.set(previous);
        assert_eq!(style, authored, "normalization is only for the current cached root");
    }

    #[test]
    fn a_scroll_container_no_longer_refuses_the_boundary() {
        set_stylesheet(".scrolls { overflow: scroll }".into());
        let (outer, plain, box_) = with_tree(|t| {
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
            (outer, plain, box_)
        });
        restyle_all();
        forget_shapes();
        assert_eq!(shape(outer), (4, false), "four nodes, and the scroll box does not block");
        assert!(!shape(plain).1, "nor does it block any ancestor of itself");
        assert!(!shape(box_).1, "nor itself");
    }

    /// The things that DO still refuse one, so removing the scroll-container exclusion cannot be
    /// read as "the predicate stopped refusing".
    #[test]
    fn a_replaced_element_still_refuses_the_boundary() {
        let (wrap, plain) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let img = t.intern("img");
            let wrap = t.create_element(div);
            let plain = t.create_element(div);
            let pic = t.create_element(img);
            t.insert(wrap, root, 0);
            t.insert(plain, wrap, 0);
            t.insert(pic, plain, 0);
            (wrap, plain)
        });
        restyle_all();
        forget_shapes();
        assert!(shape(wrap).1, "an <img> two levels down blocks");
        assert!(shape(plain).1, "and every ancestor of it");
    }

    /// The shape is memoised against the subtree revision, because deriving it is the walk the
    /// boundary exists to avoid.
    #[test]
    fn the_shape_memo_follows_the_subtree_revision() {
        set_stylesheet(".floats { position: absolute }".into());
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
        assert_eq!(shape(wrap), (2, false), "nothing is out of flow yet");

        with_tree(|t| t.set_class(inner, "floats"));
        restyle_all();
        assert!(shape(wrap).1, "the memo was invalidated by the mutation, not by a timer");
    }
}
