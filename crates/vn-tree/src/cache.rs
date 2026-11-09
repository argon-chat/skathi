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
//! * **`<input>`/`<textarea>`, `<img>`, `<video>`** — native widgets
//!   and decoders that repaint on their own clock. (`<lottie>` and
//!   `<vn-sticker>` are not: their frames are written into a tile a replayed
//!   sprite samples, and a tile that moves touches its node — `crate::lottie`,
//!   `render::sticker`);
//! * inline **`<svg>`** except the self-contained static monochrome geometry
//!   accepted by `svg::can_replay`: its alpha-mask atlas entry has no decoder
//!   clock. Colour rasters, external resources, and timelines stay excluded;
//! * an **out-of-flow box** (`position: absolute|fixed`) or a **group anchor**
//!   (`.group(name)` is registered during the paint walk);
//! * a subtree holding a node whose transition or animation is moving this
//!   frame — and only that subtree: the moving node and its ancestors are
//!   refused, every other boundary still replays (see [`begin_frame`]). A
//!   boundary below a fading or sliding node is gpui's to refuse: its view
//!   cache key holds the opacity and transform the ancestors composed (fork
//!   item 44), so it renders while they change;
//! * anything at all while a screen reader is attached — the AccessKit tree
//!   is built during the frame and is not part of the replayed spans.
//!
//! A **scroll container** used to be on that list and is the one item that came
//! off it (m23, `research/reports/m23/scroll.md`). It was refused because
//! `vn_elements::scrollbar`'s wheel-routing registry is rebuilt from the paint
//! walk and a replay does not walk, so a cached scroll box kept its pixels and
//! lost its wheel. [`Boundary`] now records the run of registry entries the
//! subtree's paint contributed and re-registers it, in place, on the frames
//! that replay. On the 1 000-row scenario the mutation frame goes from 11.5 ms
//! of work per draw to **3.6 ms** against the rest of the cache, and from
//! 89.3 ms with no cache at all — the largest single number in the
//! retained-render work, and the one this exclusion was costing.
//!
//! `VN_NO_CACHE=1` turns the whole thing off; `VN_CACHE_MIN=<n>` moves the
//! smallest subtree worth a boundary (default [`MIN_NODES`]). A boundary costs
//! about what one node costs — measured at ~18 µs against ~20 µs — so the win
//! on a subtree is its node count, and below a handful it is not worth having.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Window,
};

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
    /// The box this boundary would be pinned at, as of the last frame, or
    /// `None` on the frame the node was first seen as a candidate.
    ///
    /// It is part of the freshness key, and that is the load-bearing half of the
    /// design: while a boundary replays, its own probe records the box we PINNED
    /// rather than the box its content would take, so the pin cannot correct
    /// itself. A boundary is therefore only offered once a frame that lowered
    /// the subtree for real has measured the same box twice running.
    ///
    /// **`None` is what makes "twice running" true rather than aspirational.**
    /// Seeding a new entry with the size in hand made the first sighting
    /// compare equal to itself and offer immediately — on ONE frame's
    /// measurement, taken *before* the build that is happening now. When the
    /// two disagree (the first frame a stylesheet lands on: the tree is being
    /// lowered styled while `prev_size_of` still answers with the unstyled
    /// box), the wrong box is pinned, the replay's own probe re-records it,
    /// and nothing ever measures again. `apps/listperf` froze its whole page
    /// at the unstyled 104 px row height that way — the exact shape
    /// `research/reports/m23/perf-retained.md` §4.2 describes and the rule it
    /// names, which until m23 the code did not actually implement.
    size: Option<(f32, f32)>,
    /// Last observed raw bounds. A translated row cannot replay GPUI's
    /// bounds-keyed recording, even when its dimensions are unchanged.
    rect: Option<Rect>,
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
    // The host's chrome registry is reset every build. GPUI carries its own
    // caption hitboxes on replay, but cannot refill these SDK-side facts.
    controls: Vec<(u32, gpui::WindowControlArea)>,
    /// The run of `vn_elements::scrollbar` registry entries this subtree
    /// contributed the last time it was painted for real.
    ///
    /// The wheel router's registry is rebuilt from the PAINT walk, and a
    /// replayed subtree does not walk — which is why v1 refused a boundary
    /// over a scroll container outright (§4.4 of the report). The span is
    /// recorded by [`Boundary::paint`] around the inner element's paint and
    /// re-registered, in place, on the frames that replay. Both of an entry's
    /// halves survive: a `HitboxId` comes from a counter that is never reset,
    /// and `reuse_prepaint` copies the recorded hitbox into this frame's hit
    /// test WITH that id, so `should_handle_scroll` still answers for it.
    chain: vn_elements::ChainSpan,
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
    /// The nodes the animator is moving in the frame being built, and every
    /// ancestor of each — the boundaries [`begin_frame`] refuses.
    static HOT: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
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

/// `VN_CACHE_NO_SCROLL=1` — put the scroll-container exclusion back.
///
/// Two jobs, and the second is why it is shipped rather than deleted after the
/// measurement. It is the A/B switch the "cache on, scroll container refused"
/// column of `research/reports/m23/scroll.md` §3 was measured with, so the 3×
/// that removing the exclusion is worth stays reproducible by anyone reading
/// the report. And it is the one-flag way to answer "is this a cached scroll
/// box?" about a suspected rendering bug in the field, without giving up the
/// rest of the cache the way `VN_NO_CACHE=1` does.
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

/// Decided once per build: may anything replay in this frame, and which
/// subtrees are moving?
///
/// The two global refusals are the ones no per-node predicate can see. A
/// screen reader is the important one — the AccessKit tree is assembled during
/// the frame and is not part of the prepaint/paint spans gpui replays, so a
/// cached subtree would simply not exist for Narrator. And a frame on which an
/// image's smaller copy landed or was evicted (`crate::resample`) must repaint
/// the scenes that name its tile.
///
/// **Animation is per subtree, not per frame.** Everything animating is
/// re-sampled from the build's one `Instant`, so a replayed subtree would
/// freeze mid-transition — but only the subtree that HOLDS the moving node.
/// This used to refuse every boundary in the window while anything at all
/// moved, which in the Argon client meant never: one visible spinner and the
/// whole 4 000-node chat re-rendered at the display's rate. Now the moving
/// nodes and their ancestors are collected here ([`HOT`]) at `now`, the same
/// instant `build_node` samples the overlays at, and only a boundary on that
/// path is refused. A `<video>` never needed the global refusal: its tag
/// already blocks every boundary above it ([`node_blocks`]). A `<lottie>`
/// needs none: its frames are tile writes a replay shows.
///
/// The frame a node STOPS moving is the one that must not replay: the last
/// recording of its subtree, if there is one, predates the animation. A
/// refusal therefore marks the entry not-offered, and the first offer after
/// that mints a fresh entity, which renders — at `now`, where the animation
/// has finished — instead of replaying.
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
    // A referenced definition can change outside this subtree, so its matrix
    // must be resolved afresh; descendants still reuse the matrix-aware cache.
    if cs.extras.filter_url.is_some() { return true; }
    // A scroll container is NO LONGER refused by default. The wheel registry
    // it contributes to is rebuilt from the paint walk and a replay does not
    // walk, so the span it pushed is recorded and re-registered in place
    // instead — see [`Boundary`]. That exclusion was the last one with a known
    // fix and the most expensive one there has been (11.5 ms → 3.6 ms per draw
    // on the 1 000-row scenario); [`refuse_scroll_containers`] is the switch
    // that puts it back, for the A/B and for a field diagnosis.
    if refuse_scroll_containers()
        && (cs.base.overflow.x == Some(gpui::Overflow::Scroll)
            || cs.base.overflow.y == Some(gpui::Overflow::Scroll))
    {
        return true;
    }
    // Out of flow, or escaping its siblings' paint order: `render::stack` moves
    // the paint into gpui's deferred layer and an absolutely positioned box is
    // placed by its containing block rather than by its parent's flow, so
    // neither the pinned size nor the parent-rectangle check means what it says.
    // It also keeps every deferred draw, and so every stacking key, out of a
    // recording: a key holds tree-order ordinals that move whenever anything
    // earlier in the document does, and a negative `z-index` has to be SEEN
    // while its context's children are built (`render::Stacking`).
    if cs.base.position == Some(gpui::Position::Absolute)
        || cs.extras.may_defer()
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
    // SVG serializes its children into one leaf; counting shape nodes would
    // offer a cache wrapper for a single native element with a large XML tree.
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
        // only here. normalize_inner_root removes its inner duplicate.
        st.margin = cs.base.margin.clone();
        st.align_self = cs.base.align_self;
    }
    st
}

/// The cached view already resolved its root's outer geometry. Resolving
/// percentage widths or max-width against that pinned box a second time makes
/// an 85%-wide chat column 85% of itself; right-aligned bubbles then jump each
/// time a hover frame switches between a cached and an ordinary build.
/// Descendants retain their authored geometry, padding, borders and alignment.
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

/// The view a boundary hands to gpui. It owns nothing but the coordinates of
/// the subtree it lowers; the tree itself stays where it is.
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
    // Something inside is moving: lower it this frame, and make sure the
    // frame it stops moving renders rather than replays (see [`begin_frame`]).
    if hot(id) {
        with_entry(id, |e| e.offered = false);
        return None;
    }
    // The box the subtree laid out to last frame. Nothing to pin on the first
    // frame a node is painted, so the first frame is always a real build.
    let rect = render::prev_rect_of(id).filter(|(_, _, width, height)| *width > 0.0 && *height > 0.0)?;
    let size = (rect.2, rect.3);

    let parent_rect = (node.parent != 0).then(|| render::prev_rect_of(node.parent)).flatten();

    let sub_rev = node.sub_rev;
    // Everything that has to hold for the recording to still describe this node.
    // `size` is in there for the reason spelled out on [`Entry::size`]; the rest
    // is "nothing inside changed" (`sub_rev`), "nothing an ancestor threads down
    // changed" (`ctx`) and "nothing outside resized me" (`parent_rect`).
    // Refuse moved origins immediately once observed, without paying for three
    // futile GPUI re-renders before the ordinary thrash cooldown.
    let offer = ENTRIES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m.entry(id).or_insert_with(|| Entry {
            entity: None,
            sub_rev,
            ctx: ctx.clone(),
            parent_rect,
            // NOT `Some(size)`: the first sighting must not compare equal to
            // itself and offer on one frame's measurement — see [`Entry::size`].
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
            e.sub_rev == sub_rev
                && e.ctx == ctx
                && e.parent_rect == parent_rect
                && e.size == Some(size)
                && e.rect == Some(rect);
        if !fresh {
            // Lower it the ordinary way this frame. That is what re-measures it
            // honestly — a wrapper would pin the very box we have just decided
            // we no longer believe — and it is cheaper than rendering through
            // the boundary only to throw the recording away.
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
    // Whether gpui replays or re-renders, the rectangles this subtree recorded
    // last frame are the rectangles it has now — the replay is only taken when
    // the bounds are identical, and a re-render overwrites them anyway.
    carry_bounds(id, win);
    Some(
        Boundary { id, inner: entity.cached(wrapper_style(node, size)).into_any_element() }
            .into_any_element(),
    )
}

/// The one thing a replay has to be given back by hand: the scroll chain.
///
/// Everything else a boundary needs is either copied forward by gpui
/// (`reuse_prepaint` / `reuse_paint`) or carried by us before the element is
/// even built ([`carry_bounds`]). `vn_elements::scrollbar`'s wheel-routing
/// registry is neither: it is rebuilt from the **paint walk**, so a subtree
/// that replays contributes nothing to it and every scroll container inside it
/// falls out of the chain — the box keeps its pixels and stops scrolling. That
/// is the whole of why v1 refused to cache a scroll container.
///
/// This wrapper brackets the cached element's paint and does the bookkeeping
/// the walk would have done:
///
/// * **painted for real** — the subtree pushed its own entries, so record the
///   run as this boundary's span;
/// * **replayed** — it pushed nothing, so re-push the recorded run, *here*,
///   which is exactly the position in the walk the subtree occupied. The
///   registry is read innermost-first and that order is positional, so
///   re-registering in place is what keeps a nested box ahead of the box that
///   contains it.
///
/// It contributes no layout of its own: `request_layout` hands back the inner
/// element's `LayoutId`, so taffy sees the same one leaf it saw before.
struct Boundary {
    id: u32,
    inner: AnyElement,
}

impl IntoElement for Boundary {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
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
                // Nested cached entities render after the outer build has
                // returned. Snapshot after paint so their areas are included.
                entry.controls = windows::peek(win, |state| {
                    state.control_hitboxes.iter()
                        .filter(|(id, _)| entry.ids.contains(id)).copied().collect()
                }).unwrap_or_default();
            }
        });
        let span = vn_elements::scrollbar::chain_since(mark);
        if span.is_empty() {
            // Nothing registered: either this subtree really has no scroll
            // container (the span is empty too, and this is a no-op), or gpui
            // replayed the recording and the walk that would have registered
            // them never ran.
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
        // Nested boundaries can be rendered while an ancestor's saved span
        // has already been carried. Each node still contributes just once.
        if !state.control_hitboxes.contains(&(id, area)) {
            state.control_hitboxes.push((id, area));
        }
    }
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
/// `(cache boundaries tracked, those holding an entity, node ids they carry)`
/// — devtools `mem`. The recorded scene spans themselves live in gpui.
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
        // Match __vn_set_stylesheet: a replacement marks the document dirty.
        with_tree(|t| t.mark_dirty());
        restyle_all();
        let after = read_tree(|t| (t.sub_rev(row), t.node(svg).unwrap().computed.as_ref().unwrap().base.text.color));
        assert_ne!(before.0, after.0, "a cached ancestor must rebuild its recorded tint");
        assert_ne!(before.1, after.1);
        assert!(!shape(row).1, "currentColor tint changes do not require a colour raster");
        set_stylesheet(".icon { color: blue } .icon path { stroke: red }".into());
        // Match __vn_set_stylesheet: a replacement marks the document dirty.
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

    /// A scroll container no longer refuses the boundary — anywhere inside it,
    /// or as the boundary node itself.
    ///
    /// It was the one exclusion with a real price (11.5 ms → 3.6 ms per draw on
    /// the 1 000-row scenario) and the one with a known fix: the registry entry a
    /// scroll box contributes to the wheel router is recorded per boundary and
    /// re-registered on a replay ([`Boundary`]), rather than being rebuilt by a
    /// paint walk that does not happen.
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

    /// The things that DO still refuse one, so removing the scroll-container
    /// exclusion cannot be read as "the predicate stopped refusing".
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

    /// The shape is memoised against the subtree revision, because deriving it
    /// is the walk the boundary exists to avoid. A mutation has to be enough to
    /// re-derive it — otherwise a `<div>` that becomes a replaced element or an
    /// out-of-flow box would keep a boundary that is no longer sound.
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
