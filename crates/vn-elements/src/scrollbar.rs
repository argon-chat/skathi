//! Overlay scrollbars, and the wheel routing that makes `overscroll-behavior`
//! mean something.
//!
//! # Provenance
//!
//! Adapted from **longbridge/gpui-kit** `crates/base/src/scrollbar.rs` and
//! `crates/base/src/scrollable_mask.rs` at commit `b6f098db81` (2026-09-03),
//! Copyright 2024–2026 Longbridge, licensed under the Apache License 2.0. The
//! upstream `LICENSE-APACHE` applies to the derived parts of this file, which
//! is the same licence our own tree already carries.
//!
//! What was taken is the *technique*, not the text: the load-bearing details
//! that a from-scratch scrollbar gets wrong, listed here so a future reader can
//! tell adaptation from invention.
//!
//! * **It is a real `gpui::Element`, not a `div`.** It requests an absolutely
//!   positioned box and then ignores its own layout bounds: the geometry comes
//!   from `ScrollHandle::bounds()`, the viewport the container recorded, in
//!   window space. That is what lets the bar be painted as the scroll box's own
//!   last child — inside the subtree the scroll offset translates — and still
//!   land on the viewport's edge rather than sliding away with the content.
//! * **Two rectangles per thumb.** `thumb_hit` is the full track width and does
//!   not shrink; `thumb_fill` is what is painted and does. A thumb that widens
//!   on hover must not move its own hit target out from under the pointer.
//! * **Auto-hide is not a timer on the render loop.** A stopped scroll produces
//!   no further frames, so nothing would ever repaint the bar away. Prepaint
//!   schedules a background timer for the remaining idle time and notifies when
//!   it fires, guarded by a flag so it schedules once.
//! * **Drag is throttled by time and distance** — 120 fps and one pixel.
//! * **The wheel is intercepted in the CAPTURE phase.** This is
//!   `scrollable_mask.rs`'s job and it is `overscroll-behavior` in disguise:
//!   gpui's own scroll listener runs on bubble and does NOT stop propagation,
//!   so every ancestor scroll box scrolls at once. Capture, innermost first,
//!   with an explicit decision about whether the ancestor gets the rest, is the
//!   browser's model.
//!
//! What was *not* taken: their theme (`ActiveTheme`), their `ScalarTransition`
//! easing pair, `schemars`/`serde` derives, the `ListState` and
//! `UniformListScrollHandle` handle impls (we have one kind of scroller), and
//! the `ScrollbarTrackStyle::border_color` field they left dead.
//!
//! # Why the axis lock, and why the chain terminates
//!
//! A gesture locks to the axis it began on so a diagonal trackpad swipe cannot
//! change owners mid-stroke. And the outermost scroll box in a vue-native
//! window is always the root element — "the window IS the root" gives it
//! `overflow: scroll` unconditionally — so every wheel event reaches a mask
//! that will consume it, and no event ever falls through to gpui's own
//! all-ancestors-at-once listener.

use std::cell::Cell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    px, size, point, App, Axis, Bounds, ContentMask, Element, ElementId, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Position, ScrollHandle, ScrollEvent, Style,
    Window, WindowControlArea, fill, relative,
};

/// The track's width at `scrollbar-width: auto`. Windows 11's overlay
/// scrollbar occupies about this when it is not hovered.
const TRACK_AUTO: Pixels = px(14.);
/// …and at `scrollbar-width: thin`. CSS gives no number; every engine picks
/// something around half.
const TRACK_THIN: Pixels = px(8.);
/// The gap between the thumb and the track's edges.
const INSET: Pixels = px(3.);
/// A thumb never becomes a dot, however long the content is.
const MIN_THUMB: Pixels = px(28.);
/// How long the bar stays up after the last scroll.
const IDLE: Duration = Duration::from_millis(1200);
/// How long it takes to fade away once the idle hold expires.
const FADE: Duration = Duration::from_millis(220);
/// How long the thumb takes to reach its hovered width.
const EXPAND: Duration = Duration::from_millis(120);
/// Drag updates are clamped to this, and to one pixel of movement.
const DRAG_FPS: u64 = 120;

/// `overscroll-behavior`, as this element needs it: does a wheel this box
/// cannot use reach the ancestors?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Overscroll {
    pub x_chains: bool,
    pub y_chains: bool,
}

impl Overscroll {
    /// CSS's initial value on both axes: the scroll chains.
    pub const CHAINING: Self = Self { x_chains: true, y_chains: true };

    fn chains(self, axis: Axis) -> bool {
        match axis {
            Axis::Horizontal => self.x_chains,
            Axis::Vertical => self.y_chains,
        }
    }
}

/// Everything CSS gets to say about how the bar looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarStyle {
    /// `scrollbar-width`. `None` here is `scrollbar-width: none` — the box
    /// still scrolls and still routes the wheel, it just draws nothing.
    pub track_width: Option<Pixels>,
    /// The thumb at rest.
    pub thumb: Hsla,
    /// The thumb while the pointer is over the track or dragging it.
    pub thumb_active: Hsla,
    /// The track. Usually fully transparent — an overlay scrollbar reserves no
    /// space and paints no gutter until you reach for it.
    pub track: Hsla,
    /// The track while the pointer is over it.
    pub track_active: Hsla,
    /// Ignore the idle timer and keep the bar up. This is what
    /// `should_auto_hide_scrollbars() == false` asks for.
    pub always_visible: bool,
}

impl ScrollbarStyle {
    /// The default overlay look, derived from the element's own text colour.
    ///
    /// A scrollbar has to read against whatever the app painted behind it, and
    /// the one value that reliably contrasts with an element's background is
    /// the colour it draws its text in. So the thumb is that colour at low
    /// alpha: dark text (light page) gives a dark thumb, light text (dark page)
    /// a light one, with no theme, no `prefers-color-scheme` plumbing and no
    /// guess. `scrollbar-color` overrides it outright.
    pub fn from_text_color(color: Hsla) -> Self {
        Self {
            track_width: Some(TRACK_AUTO),
            thumb: Hsla { a: 0.32, ..color },
            thumb_active: Hsla { a: 0.62, ..color },
            track: Hsla { a: 0.0, ..color },
            track_active: Hsla { a: 0.07, ..color },
            always_visible: false,
        }
    }

    /// `scrollbar-color: <thumb> <track>`.
    pub fn with_colors(mut self, thumb: Hsla, track: Hsla) -> Self {
        self.thumb = thumb;
        // The author asked for one thumb colour, so the hovered state is the
        // same colour rather than a different one they did not choose.
        self.thumb_active = thumb;
        self.track = track;
        self.track_active = track;
        self
    }

    pub fn thin(mut self) -> Self {
        self.track_width = Some(TRACK_THIN);
        self
    }

    pub fn hidden(mut self) -> Self {
        self.track_width = None;
        self
    }

    pub fn always_visible(mut self, always: bool) -> Self {
        self.always_visible = always;
        self
    }
}

/// The bit of state that has to outlive a frame: what the pointer is doing, and
/// when the content last moved.
#[derive(Clone, Copy, Debug)]
struct State {
    hovered: Option<Axis>,
    dragging: Option<Axis>,
    /// Where inside the thumb the drag started.
    grab: Pixels,
    last_offset: Point<Pixels>,
    last_scroll: Option<Instant>,
    last_drag_update: Instant,
    idle_timer: bool,
    /// The axis this wheel gesture locked to, and when it was last fed. A
    /// precise (trackpad) delta keeps its axis for the whole stroke.
    lock: Option<(Axis, Instant)>,
    /// Animated track widths, per axis, so a hover widens rather than jumps.
    width: [Pixels; 2],
    width_at: Instant,
    /// Fade: the opacity the bar was at when the last visibility change began.
    fade_from: f32,
    fade_at: Instant,
    visible: bool,
}

impl Default for State {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            hovered: None,
            dragging: None,
            grab: Pixels::ZERO,
            last_offset: point(px(0.), px(0.)),
            last_scroll: None,
            last_drag_update: now,
            idle_timer: false,
            lock: None,
            width: [Pixels::ZERO; 2],
            width_at: now,
            fade_from: 0.0,
            fade_at: now,
            visible: false,
        }
    }
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// A wheel gesture is one stroke while its events keep arriving; this much
/// silence ends it and frees the axis lock.
const GESTURE_GAP: Duration = Duration::from_millis(180);

/// One painted scroll container, as the wheel router needs to see it.
#[derive(Clone)]
struct ChainEntry {
    hitbox: gpui::HitboxId,
    handle: ScrollHandle,
    scrolls_x: bool,
    scrolls_y: bool,
}

thread_local! {
    /// Every scroll container painted in the last frame, INNERMOST FIRST.
    ///
    /// A scrollbar is its container's last child, so an inner container's
    /// scrollbar paints before an outer one's — the order falls out of the
    /// paint walk and does not have to be computed.
    ///
    /// This exists for one question, which gpui cannot answer on its own:
    /// `Hitbox::should_handle_scroll` is true for EVERY hitbox under the
    /// pointer, ancestors included, and gpui's own wheel listener does not stop
    /// propagation — so a wheel over a nested box scrolls the box AND the page.
    /// A browser scrolls one of them.
    static CHAIN: std::cell::RefCell<Vec<ChainEntry>> = const { std::cell::RefCell::new(Vec::new()) };

    /// Every scroll container under the pointer, with its offset as the
    /// CAPTURE phase of the wheel event being dispatched found it: before gpui's
    /// own bubble-phase listener moved anything. Cleared by an effect at the
    /// end of the dispatch. [`undo_wheel_scroll`] puts them back.
    static BEFORE_WHEEL: std::cell::RefCell<Vec<(gpui::HitboxId, ScrollHandle, Point<Pixels>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Puts every scroll container under the pointer back where the wheel event
/// being dispatched found it, and says whether one had moved.
///
/// A browser fires the DOM `wheel` event BEFORE it scrolls, and a cancelled one
/// scrolls nothing. Here gpui's own listener on a scroll container runs in the
/// bubble phase before the listener on the same box that fires `wheel`, and the
/// listeners of any container inside it run earlier still. So the embedder
/// calls this when `wheel` was cancelled, and stops propagation so that no
/// outer container moves. Only meaningful during that wheel event's dispatch.
pub fn undo_wheel_scroll() -> bool {
    BEFORE_WHEEL.with(|b| {
        let mut moved = false;
        for (_, handle, offset) in b.borrow().iter() {
            if handle.offset() != *offset {
                handle.set_offset(*offset);
                moved = true;
            }
        }
        moved
    })
}

thread_local! {
    /// What the pointer is doing to each scrollbar, and when its content last
    /// moved — keyed by the bar's own `ElementId`, which is the scroll
    /// container's node id.
    ///
    /// **Deliberately not gpui element state.** `Window::use_state` is keyed by
    /// the `GlobalElementId` — the whole path of element ids down to this one —
    /// and a retained-render boundary CHANGES that path: the first offer after
    /// an ordinary build mints a fresh `Entity`, a fresh `EntityId` is a fresh
    /// `ElementId::View`, and every descendant's global id moves with it. The
    /// state that comes back is then a default one.
    ///
    /// For most elements that is invisible. For this one it ended a drag
    /// mid-gesture: a boundary re-rendered three frames running stands down
    /// (`vn_tree::cache`'s thrash backoff) and is re-offered ~24 frames later
    /// with a new entity, which is well inside the time a hand takes to drag a
    /// thumb. `scrollbar.spec.ts`'s "dragging the thumb scrolls the box" is the
    /// pin. The bar's identity is the container it belongs to, not its position
    /// in an element tree that is rebuilt every frame, so that is what it is
    /// keyed by.
    static STATES: std::cell::RefCell<HashMap<u64, (u64, Rc<Cell<State>>)>> =
        std::cell::RefCell::new(HashMap::new());
    /// Builds since the process started — the clock [`STATES`] is swept against.
    static FRAME: Cell<u64> = const { Cell::new(0) };
}

/// A stable key for one scrollbar: its `ElementId`, hashed. The renderer builds
/// that id as `("vn-scrollbar", <node id>)`, so it is the container's identity
/// and nothing else.
fn state_key(id: &ElementId) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

fn state_for(id: &ElementId) -> Rc<Cell<State>> {
    let key = state_key(id);
    let frame = FRAME.with(|c| c.get());
    STATES.with(|m| {
        let mut m = m.borrow_mut();
        let slot = m.entry(key).or_insert_with(|| (frame, Rc::new(Cell::new(State::default()))));
        slot.0 = frame;
        slot.1.clone()
    })
}

/// How many builds a scrollbar may go unseen before its state is dropped.
/// Generous on purpose — a `<KeepAlive>`d page comes back, and the state is
/// two pointers — and swept in the same batches, so the sweep is amortised to
/// nothing.
const STATE_TTL: u64 = 4096;

/// Drops the last frame's scroll-container registry. Called from the renderer
/// at the top of each element-tree build.
pub fn reset_frame() {
    CHAIN.with(|c| c.borrow_mut().clear());
    let frame = FRAME.with(|c| {
        let n = c.get().wrapping_add(1);
        c.set(n);
        n
    });
    if frame % STATE_TTL == 0 {
        STATES.with(|m| m.borrow_mut().retain(|_, (seen, _)| frame.saturating_sub(*seen) < STATE_TTL));
    }
}

/// A contiguous run of registry entries — what one retained-render boundary
/// contributed to the chain while it was painted for real.
///
/// The registry is rebuilt from the paint walk, and a replayed subtree does not
/// walk: gpui copies its recorded hitboxes, mouse listeners and scene forward
/// and never calls `paint` again. Without this the wheel router simply loses
/// every scroll container inside a cached subtree — the box keeps its pixels
/// and stops scrolling (research/reports/m23/perf-retained.md §4.4), which is
/// why v1 refused to cache a scroll container at all.
///
/// A span is safe to replay because **both** of its fields outlive the frame
/// they were recorded in:
///
/// * `HitboxId` is allocated from a counter that is never reset
///   (`Window::next_hitbox_id`, initialised once per window), so an id is
///   unique for the life of the window and cannot come to mean a different
///   hitbox later; and `reuse_prepaint` clones the recorded hitboxes into the
///   new frame **with their ids**, so the very hitbox this entry names is in
///   this frame's hit test.
/// * `ScrollHandle` is an `Rc` the embedder keeps per node, so the offset the
///   router reads and moves is the live one either way.
///
/// The embedder records a span with [`chain_mark`] + [`chain_since`] around the
/// boundary's paint and re-registers it with [`chain_replay`] on the frames
/// that replay. Order is preserved, and the span is re-pushed at the point in
/// the walk the subtree occupied, so "innermost first" still holds.
#[derive(Clone, Default)]
pub struct ChainSpan(Vec<ChainEntry>);

impl ChainSpan {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::fmt::Debug for ChainSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainSpan").field("entries", &self.0.len()).finish()
    }
}

/// `mark` / `since` / `replay` over a `Vec`, with no gpui in it.
///
/// Split out from [`CHAIN`] for one reason: a `HitboxId` cannot be fabricated
/// outside gpui's own `test-support` feature, so the span mechanics would
/// otherwise be the one part of this file with no unit test. They are the part
/// a replay depends on being exactly right, so they are generic and tested
/// below over `u32`.
fn span_since<T: Clone>(v: &[T], mark: usize) -> Vec<T> {
    v.get(mark..).map(<[T]>::to_vec).unwrap_or_default()
}

/// How many scroll containers have registered in this frame so far — the mark
/// a boundary takes before it paints.
pub fn chain_mark() -> usize {
    CHAIN.with(|c| c.borrow().len())
}

/// The entries registered since `mark`, in registration order.
pub fn chain_since(mark: usize) -> ChainSpan {
    CHAIN.with(|c| ChainSpan(span_since(&c.borrow(), mark)))
}

/// Re-registers a recorded span at the current point in the walk.
pub fn chain_replay(span: &ChainSpan) {
    if span.0.is_empty() {
        return;
    }
    CHAIN.with(|c| c.borrow_mut().extend(span.0.iter().cloned()));
}

/// How many scroll containers are registered in this frame. The embedder's
/// regression pin for "a replayed subtree still routes the wheel".
pub fn chain_count() -> usize {
    chain_mark()
}

/// Can this box still move on `axis` in the direction `delta` points?
fn can_move(handle: &ScrollHandle, axis: Axis, delta: Pixels) -> bool {
    let max = handle.max_offset();
    let offset = handle.offset();
    let (current, limit) = match axis {
        Axis::Vertical => (offset.y, max.y),
        Axis::Horizontal => (offset.x, max.x),
    };
    if limit <= px(0.5) {
        return false;
    }
    let next = (current + delta).clamp(-limit, px(0.));
    (next - current).abs() > px(0.01)
}

/// An overlay scrollbar for one `overflow: auto | scroll` box, plus the wheel
/// routing for it.
///
/// Paint it as the scroll container's LAST child. It contributes nothing to
/// layout (it is absolutely positioned) and reads its geometry from the handle,
/// so it does not care where in the subtree it ends up.
pub struct Scrollbar {
    id: ElementId,
    handle: ScrollHandle,
    /// Which axes the container's `overflow` actually scrolls.
    axes: (bool, bool),
    style: ScrollbarStyle,
    overscroll: Overscroll,
    line_height: Pixels,
    on_wheel: Option<Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App) -> bool>>,
    on_scrolled: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl Scrollbar {
    pub fn new(id: impl Into<ElementId>, handle: ScrollHandle, style: ScrollbarStyle) -> Self {
        Self {
            id: id.into(),
            handle,
            axes: (true, true),
            style,
            overscroll: Overscroll::CHAINING,
            line_height: px(16.),
            on_wheel: None,
            on_scrolled: None,
        }
    }

    /// Dispatch the DOM `wheel` event for a gesture this element takes over.
    /// Returning `true` is `preventDefault()`: the box does not scroll.
    ///
    /// Only called on the takeover path — when the router has decided that
    /// letting gpui's own listener run would scroll an ancestor as well. Every
    /// other wheel goes through the renderer's ordinary listener untouched.
    pub fn on_wheel(
        mut self,
        f: impl Fn(Point<Pixels>, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.on_wheel = Some(Rc::new(f));
        self
    }

    /// Report a scroll this element performed itself — a taken-over wheel, a
    /// thumb drag, a click on the track. The DOM `scroll` event.
    pub fn on_scrolled(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_scrolled = Some(Rc::new(f));
        self
    }

    /// Which axes this box scrolls — straight off the computed `overflow`.
    pub fn axes(mut self, x: bool, y: bool) -> Self {
        self.axes = (x, y);
        self
    }

    pub fn overscroll(mut self, overscroll: Overscroll) -> Self {
        self.overscroll = overscroll;
        self
    }

    /// The line height a non-precise (mouse) wheel notch is measured in.
    pub fn line_height(mut self, line_height: Pixels) -> Self {
        self.line_height = line_height;
        self
    }

    fn wants(&self, axis: Axis) -> bool {
        match axis {
            Axis::Horizontal => self.axes.0,
            Axis::Vertical => self.axes.1,
        }
    }
}

/// Everything one axis needs at paint time.
struct AxisPaint {
    axis: Axis,
    /// The whole track, at the viewport's edge.
    track: Bounds<Pixels>,
    /// The full-width hit target.
    thumb_hit: Bounds<Pixels>,
    /// The (possibly narrower) painted thumb.
    thumb_fill: Bounds<Pixels>,
    /// The track hitbox, for hover and for opting out of a `window-drag` region.
    hitbox: Hitbox,
    /// Content minus viewport on this axis — the distance the offset can cover.
    max_offset: Pixels,
    /// The distance the thumb can travel.
    thumb_travel: Pixels,
}

pub struct Prepaint {
    viewport: Option<Hitbox>,
    axes: Vec<AxisPaint>,
    state: Rc<Cell<State>>,
    opacity: f32,
}

impl IntoElement for Scrollbar {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Scrollbar {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Absolute, so it is out of flow and cannot become a flex item of the
        // box it overlays. The size is irrelevant — `prepaint` reads the
        // viewport off the handle — but a zero-size absolute box is what taffy
        // spends the least time on.
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.size.width = relative(0.).into();
        style.size.height = relative(0.).into();
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = state_for(&self.id);
        let now = Instant::now();

        // The viewport, in window space, as the container recorded it. NOT this
        // element's own layout bounds: those ride the scroll offset.
        let viewport = self.handle.bounds();
        if viewport.size.width <= px(0.) || viewport.size.height <= px(0.) {
            return Prepaint { viewport: None, axes: Vec::new(), state, opacity: 0.0 };
        }

        let mut inner = state.get();
        let offset = self.handle.offset();
        if offset != inner.last_offset {
            inner.last_offset = offset;
            inner.last_scroll = Some(now);
        }

        // Visibility, and the timer that is the only thing able to take it
        // away: a stopped scroll produces no more frames on its own.
        let held = inner
            .last_scroll
            .is_some_and(|t| now.saturating_duration_since(t) < IDLE);
        let want = self.style.always_visible || held || inner.hovered.is_some() || inner.dragging.is_some();
        if want != inner.visible {
            inner.fade_from = sample_opacity(&inner, now);
            inner.fade_at = now;
            inner.visible = want;
        }
        let opacity = sample_opacity(&inner, now);
        if (opacity - if inner.visible { 1.0 } else { 0.0 }).abs() > f32::EPSILON {
            window.request_animation_frame();
        }
        if want && !self.style.always_visible && inner.hovered.is_none() && inner.dragging.is_none() {
            if let Some(last) = inner.last_scroll {
                let elapsed = now.saturating_duration_since(last);
                if elapsed < IDLE && !inner.idle_timer {
                    inner.idle_timer = true;
                    let state = state.clone();
                    let view = window.current_view();
                    let delay = IDLE - elapsed;
                    window
                        .spawn(cx, async move |cx| {
                            cx.background_executor().timer(delay).await;
                            let mut s = state.get();
                            s.idle_timer = false;
                            state.set(s);
                            cx.update(|_, cx| cx.notify(view)).ok();
                        })
                        .detach();
                }
            }
        }
        state.set(inner);

        let max = self.handle.max_offset();
        let track_width = self.style.track_width;
        let both = track_width.is_some()
            && self.wants(Axis::Vertical)
            && self.wants(Axis::Horizontal)
            && max.y > px(0.5)
            && max.x > px(0.5);

        let hitbox = window
            .with_content_mask(Some(ContentMask::rect(viewport)), |window| {
                window.insert_hitbox(viewport, HitboxBehavior::Normal)
            });

        let mut axes = Vec::new();
        if let Some(width) = track_width {
            for axis in [Axis::Vertical, Axis::Horizontal] {
                if !self.wants(axis) {
                    continue;
                }
                let vertical = axis == Axis::Vertical;
                let (max_offset, viewport_len) = if vertical {
                    (max.y, viewport.size.height)
                } else {
                    (max.x, viewport.size.width)
                };
                // Nothing to scroll on this axis: no bar, exactly as in a
                // browser.
                if max_offset <= px(0.5) {
                    continue;
                }
                // The horizontal track stops short of the vertical one, so the
                // two thumbs never meet in the corner.
                let margin_end = if both && !vertical { width } else { px(0.) };
                let track = if vertical {
                    Bounds {
                        origin: point(viewport.origin.x + viewport.size.width - width, viewport.origin.y),
                        size: size(width, viewport.size.height),
                    }
                } else {
                    Bounds {
                        origin: point(viewport.origin.x, viewport.origin.y + viewport.size.height - width),
                        size: size(viewport.size.width, width),
                    }
                };

                let track_len = if vertical { track.size.height } else { track.size.width } - margin_end;
                let content = viewport_len + max_offset;
                // The browser's proportion: thumb is to track as viewport is to
                // content, floored so it never becomes a dot.
                let thumb_len = (track_len * (viewport_len / content)).max(MIN_THUMB).min(track_len);
                let travel = track_len - thumb_len;
                let scrolled = if vertical { -offset.y } else { -offset.x };
                let start = (travel * (scrolled / max_offset).clamp(0., 1.)).max(px(0.));

                // Two rectangles: the hit target is the full track width and
                // never moves; the fill is what the hover animation narrows.
                let hovered = state.get().hovered == Some(axis) || state.get().dragging == Some(axis);
                let target = if hovered { width - INSET * 2. } else { (width - INSET * 2.) * 0.66 };
                let mut s = state.get();
                let idx = if vertical { 0 } else { 1 };
                let fill_w = animate_width(&mut s, idx, target, now);
                state.set(s);
                if (fill_w - target).abs() > px(0.1) {
                    window.request_animation_frame();
                }

                let (thumb_hit, thumb_fill) = if vertical {
                    (
                        Bounds {
                            origin: point(track.origin.x, track.origin.y + start),
                            size: size(width, thumb_len),
                        },
                        Bounds {
                            origin: point(
                                track.origin.x + (width - fill_w) / 2.,
                                track.origin.y + start,
                            ),
                            size: size(fill_w, thumb_len),
                        },
                    )
                } else {
                    (
                        Bounds {
                            origin: point(track.origin.x + start, track.origin.y),
                            size: size(thumb_len, width),
                        },
                        Bounds {
                            origin: point(
                                track.origin.x + start,
                                track.origin.y + (width - fill_w) / 2.,
                            ),
                            size: size(thumb_len, fill_w),
                        },
                    )
                };

                let track_hitbox = window
                    .with_content_mask(Some(ContentMask::rect(viewport)), |window| {
                        window.insert_hitbox(track, HitboxBehavior::Normal)
                    });
                axes.push(AxisPaint {
                    axis,
                    track,
                    thumb_hit,
                    thumb_fill,
                    hitbox: track_hitbox,
                    max_offset,
                    thumb_travel: travel,
                });
            }
        }

        Prepaint { viewport: Some(hitbox), axes, state, opacity }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(viewport) = prepaint.viewport.clone() else { return };
        let view = window.current_view();
        let state = prepaint.state.clone();
        let opacity = prepaint.opacity;
        let bounds = viewport.bounds;

        // ---- the scroll chain -----------------------------------------
        //
        // Register this container, then route the wheel. Both halves exist for
        // one gpui fact: `should_handle_scroll` is true for every hitbox under
        // the pointer, and gpui's own wheel listener does not stop propagation
        // - so a wheel over a nested box moves the box AND the page. A browser
        // moves the innermost one that can move, and consults
        // `overscroll-behavior` before it moves anything else.
        //
        // The router only takes over when it has to: when this box is the
        // innermost scrollable under the pointer AND some ancestor scrollable
        // could also move. Everything else goes through gpui's listener
        // untouched, which is what keeps the renderer's own `wheel` / `scroll`
        // listeners - and `preventDefault` - behaving exactly as they did.
        CHAIN.with(|c| {
            c.borrow_mut().push(ChainEntry {
                hitbox: viewport.id,
                handle: self.handle.clone(),
                scrolls_x: self.axes.0,
                scrolls_y: self.axes.1,
            })
        });
        {
            let handle = self.handle.clone();
            let state = state.clone();
            let overscroll = self.overscroll;
            let (wants_x, wants_y) = self.axes;
            let line_height = self.line_height;
            let hitbox_id = viewport.id;
            let on_wheel = self.on_wheel.clone();
            let on_scrolled = self.on_scrolled.clone();
            window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
                if !phase.capture() || !hitbox_id.should_handle_scroll(window) {
                    return;
                }
                // Where this box stood before anything moved it (see
                // `undo_wheel_scroll`), once per container per event.
                BEFORE_WHEEL.with(|b| {
                    let mut b = b.borrow_mut();
                    if b.is_empty() {
                        cx.defer(|_| BEFORE_WHEEL.with(|b| b.borrow_mut().clear()));
                    }
                    if !b.iter().any(|(id, _, _)| *id == hitbox_id) {
                        b.push((hitbox_id, handle.clone(), handle.offset()));
                    }
                });
                let now = Instant::now();
                let raw = event.delta.pixel_delta(line_height);
                let mut s = state.get();

                // Axis lock, per GESTURE rather than per event: a diagonal
                // trackpad swipe must not change owners halfway through.
                let axis = match s.lock {
                    Some((axis, at)) if now.saturating_duration_since(at) < GESTURE_GAP => axis,
                    _ => {
                        if raw.y.abs() >= raw.x.abs() {
                            Axis::Vertical
                        } else {
                            Axis::Horizontal
                        }
                    }
                };
                s.lock = Some((axis, now));
                state.set(s);

                // gpui maps a vertical delta onto the horizontal axis for a box
                // that only scrolls that way, and so do we - that is what a
                // browser does with a plain mouse wheel over a carousel.
                let (axis, delta) = match axis {
                    Axis::Vertical if !wants_y && wants_x => (Axis::Horizontal, raw.y),
                    Axis::Vertical => (Axis::Vertical, raw.y),
                    Axis::Horizontal if !wants_x && wants_y => (Axis::Vertical, raw.x),
                    Axis::Horizontal => (Axis::Horizontal, raw.x),
                };

                // Am I the innermost scrollable under the pointer, and is there
                // an outer one that would also move? Both answers come out of
                // one walk of the registry, which is innermost-first.
                let (innermost, outer_moves) = CHAIN.with(|c| {
                    let chain = c.borrow();
                    let mut seen_self = false;
                    let mut innermost = true;
                    let mut outer_moves = false;
                    for entry in chain.iter() {
                        if !entry.hitbox.should_handle_scroll(window) {
                            continue;
                        }
                        if entry.hitbox == hitbox_id {
                            seen_self = true;
                            continue;
                        }
                        if !seen_self {
                            // A scrollable painted before me and under the
                            // pointer is nested inside me.
                            innermost = false;
                            break;
                        }
                        let usable = match axis {
                            Axis::Vertical => entry.scrolls_y,
                            Axis::Horizontal => entry.scrolls_x,
                        };
                        if usable && can_move(&entry.handle, axis, delta) {
                            outer_moves = true;
                            break;
                        }
                    }
                    (innermost, outer_moves)
                });
                if !innermost || !outer_moves {
                    return;
                }

                let usable = match axis {
                    Axis::Vertical => wants_y,
                    Axis::Horizontal => wants_x,
                };
                // Take over only when taking over CHANGES something — that is,
                // only when this handler is going to stop propagation. If this
                // box is already at its edge on the locked axis and the axis
                // chains, the outcome is the one gpui's own listener produces
                // anyway (the ancestors move, this box does not), and the DOM
                // `wheel` belongs to the ordinary bubble path.
                //
                // This is not an optimisation. Dispatching `wheel` here and
                // then letting the event carry on to the bubble phase would
                // fire the app's `@wheel` handler TWICE for one physical
                // wheel — once from `on_wheel` below and once from the
                // renderer's own `on_scroll` listener on this same node.
                let will_stop = (usable && can_move(&handle, axis, delta))
                    || !overscroll.chains(axis);
                if !will_stop {
                    return;
                }

                // Taking over. The DOM `wheel` event first, because it is
                // cancelable and a browser fires it before it scrolls anything.
                let prevented = on_wheel
                    .as_ref()
                    .map(|f| f(dom_delta(raw), window, cx))
                    .unwrap_or(false);

                let mut moved = false;
                if !prevented && usable {
                    let max = handle.max_offset();
                    let offset = handle.offset();
                    let (current, limit) = match axis {
                        Axis::Vertical => (offset.y, max.y),
                        Axis::Horizontal => (offset.x, max.x),
                    };
                    // gpui's offsets are <= 0; CSS scrollTop counts the other way.
                    let next = (current + delta).clamp(-limit, px(0.));
                    if limit > px(0.5) && (next - current).abs() > px(0.01) {
                        let mut o = offset;
                        match axis {
                            Axis::Vertical => o.y = next,
                            Axis::Horizontal => o.x = next,
                        }
                        handle.set_offset(o);
                        let mut s = state.get();
                        s.last_offset = o;
                        s.last_scroll = Some(now);
                        state.set(s);
                        moved = true;
                    }
                }
                if moved {
                    if let Some(f) = &on_scrolled {
                        f(window, cx);
                    }
                    cx.notify(view);
                }
                // The chaining decision, which is the whole of
                // `overscroll-behavior`: an event this box could not use goes
                // on to the ancestors only when the axis chains. When it does,
                // gpui's own listener runs and moves them - and moves this box
                // by nothing, because it is already at its edge.
                if moved || prevented || !overscroll.chains(axis) {
                    cx.stop_propagation();
                }
            });
        }

        if prepaint.axes.is_empty() || opacity <= 0.001 {
            return;
        }

        window.with_content_mask(Some(ContentMask::rect(bounds)), |window| {
            for ax in prepaint.axes.iter() {
                let axis = ax.axis;
                // A titlebar that declared `window-drag: move` and happens to
                // reach the viewport's edge would otherwise have Win32 start a
                // window drag when the thumb is grabbed — the press would never
                // reach us at all, because the answer to `WM_NCHITTEST` is
                // given before any mouse message is generated. `NoDrag` is the
                // positive "this is client content" answer that opts back out.
                // Paint-phase only: gpui asserts on the frame it is writing to.
                window.insert_window_control_hitbox(
                    WindowControlArea::NoDrag,
                    ax.hitbox.clone(),
                );
                let hovered = state.get().hovered == Some(axis) || state.get().dragging == Some(axis);
                let track_bg = if hovered { self.style.track_active } else { self.style.track };
                let thumb_bg = if hovered { self.style.thumb_active } else { self.style.thumb };
                let radius = (ax.thumb_fill.size.width.min(ax.thumb_fill.size.height)) / 2.;

                window.paint_layer(bounds, |window| {
                    window.paint_quad(fill(ax.track, faded(track_bg, opacity)));
                    window.paint_quad(
                        fill(ax.thumb_fill, faded(thumb_bg, opacity)).corner_radii(radius),
                    );
                });

                // ---- press: grab the thumb, or jump the track --------------
                {
                    let handle = self.handle.clone();
                    let state = state.clone();
                    let track = ax.track;
                    let thumb_hit = ax.thumb_hit;
                    let max_offset = ax.max_offset;
                    let travel = ax.thumb_travel;
                    let hitbox = ax.hitbox.clone();
                    let on_scrolled = self.on_scrolled.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if !phase.bubble() || !hitbox.is_hovered(window) || !track.contains(&event.position) {
                            return;
                        }
                        cx.stop_propagation();
                        let vertical = axis == Axis::Vertical;
                        let mut s = state.get();
                        let mut jumped = false;
                        if thumb_hit.contains(&event.position) {
                            s.grab = if vertical {
                                event.position.y - thumb_hit.origin.y
                            } else {
                                event.position.x - thumb_hit.origin.x
                            };
                        } else {
                            // A press on the bare track centres the thumb on
                            // the pointer and starts dragging from there, which
                            // is what every overlay scrollbar does.
                            let half = if vertical { thumb_hit.size.height } else { thumb_hit.size.width } / 2.;
                            s.grab = half;
                            let along = if vertical {
                                event.position.y - track.origin.y
                            } else {
                                event.position.x - track.origin.x
                            };
                            let fraction = if travel > px(0.) {
                                ((along - half) / travel).clamp(0., 1.)
                            } else {
                                0.
                            };
                            let mut o = handle.offset();
                            let next = -max_offset * fraction;
                            if vertical { o.y = next } else { o.x = next }
                            handle.set_offset(o);
                            s.last_offset = o;
                            jumped = true;
                        }
                        s.dragging = Some(axis);
                        s.last_scroll = Some(Instant::now());
                        state.set(s);
                        // A click on the bare track moves the content, so it is
                        // a `scroll` event like any other.
                        if jumped {
                            if let Some(f) = &on_scrolled {
                                f(window, cx);
                            }
                        }
                        cx.notify(view);
                    });
                }

                // ---- move: hover, and the drag ----------------------------
                {
                    let handle = self.handle.clone();
                    let state = state.clone();
                    let track = ax.track;
                    let thumb_hit = ax.thumb_hit;
                    let max_offset = ax.max_offset;
                    let travel = ax.thumb_travel;
                    let hitbox = ax.hitbox.clone();
                    let on_scrolled = self.on_scrolled.clone();
                    let throttle = Duration::from_millis(1000 / DRAG_FPS);
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if !phase.bubble() {
                            return;
                        }
                        let mut notify = false;
                        let mut dragged = false;
                        let mut s = state.get();
                        let over = hitbox.is_hovered(window) && track.contains(&event.position);
                        if over && s.hovered != Some(axis) {
                            s.hovered = Some(axis);
                            s.last_scroll = Some(Instant::now());
                            notify = true;
                        } else if !over && s.hovered == Some(axis) {
                            s.hovered = None;
                            notify = true;
                        }
                        // Our own `dragging`, not gpui's `event.dragging()`:
                        // the press is what starts a thumb drag, and a
                        // synthesized move (the devtools' `mouse`, a tablet)
                        // carries no `pressed_button` even mid-gesture. We
                        // saw the press; that is the fact that matters.
                        if s.dragging == Some(axis) {
                            cx.stop_propagation();
                            let vertical = axis == Axis::Vertical;
                            let along = if vertical {
                                event.position.y - track.origin.y
                            } else {
                                event.position.x - track.origin.x
                            };
                            let fraction = if travel > px(0.) {
                                ((along - s.grab) / travel).clamp(0., 1.)
                            } else {
                                0.
                            };
                            let next = -max_offset * fraction;
                            let current = if vertical { handle.offset().y } else { handle.offset().x };
                            // Throttled by time AND by distance, so a drag on a
                            // long document does not repaint per mickey.
                            if (next - current).abs() > px(1.)
                                && Instant::now().saturating_duration_since(s.last_drag_update) >= throttle
                            {
                                let mut o = handle.offset();
                                if vertical { o.y = next } else { o.x = next }
                                handle.set_offset(o);
                                s.last_offset = o;
                                s.last_drag_update = Instant::now();
                                s.last_scroll = Some(Instant::now());
                                notify = true;
                                dragged = true;
                            }
                        }
                        let _ = thumb_hit;
                        state.set(s);
                        if dragged {
                            if let Some(f) = &on_scrolled {
                                f(window, cx);
                            }
                        }
                        if notify {
                            cx.notify(view);
                        }
                    });
                }

                // ---- release ----------------------------------------------
                {
                    let state = state.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if !phase.bubble() {
                            return;
                        }
                        let mut s = state.get();
                        if s.dragging == Some(axis) {
                            s.dragging = None;
                            s.last_scroll = Some(Instant::now());
                            state.set(s);
                            cx.notify(view);
                        }
                    });
                }
            }
        });
        let _ = cx;
    }
}

/// gpui's wheel delta is the platform's (positive = the wheel turned away from
/// you = the content moves DOWN); the DOM's is the opposite.
fn dom_delta(delta: Point<Pixels>) -> Point<Pixels> {
    point(-delta.x, -delta.y)
}

fn faded(color: Hsla, opacity: f32) -> Hsla {
    Hsla { a: color.a * opacity, ..color }
}

fn sample_opacity(state: &State, now: Instant) -> f32 {
    let target = if state.visible { 1.0 } else { 0.0 };
    if FADE.is_zero() {
        return target;
    }
    let t = now.saturating_duration_since(state.fade_at).as_secs_f32() / FADE.as_secs_f32();
    if t >= 1.0 {
        target
    } else {
        state.fade_from + (target - state.fade_from) * ease_out(t)
    }
}

/// Moves one axis's painted thumb width toward `target`, and reports where it
/// got to. Pure: `now` is injected, which is what makes it testable.
fn animate_width(state: &mut State, idx: usize, target: Pixels, now: Instant) -> Pixels {
    let from = state.width[idx];
    if from == Pixels::ZERO {
        // First frame: adopt rather than animate in from nothing.
        state.width[idx] = target;
        state.width_at = now;
        return target;
    }
    if (from - target).abs() < px(0.1) {
        state.width[idx] = target;
        return target;
    }
    let t = now.saturating_duration_since(state.width_at).as_secs_f32() / EXPAND.as_secs_f32();
    let next = if t >= 1.0 { target } else { from + (target - from) * ease_out(t) };
    if t >= 1.0 || (next - from).abs() > px(0.05) {
        state.width[idx] = next;
        state.width_at = now;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The browser's proportion: a viewport that shows a quarter of the content
    /// gets a thumb a quarter of the track long, and one that shows almost all
    /// of it gets a nearly full one.
    fn thumb_len(viewport: f32, content: f32, track: f32) -> f32 {
        let t = px(track) * (px(viewport) / px(content));
        f32::from(t.max(MIN_THUMB).min(px(track)))
    }

    #[test]
    fn the_thumb_is_proportional_and_never_a_dot() {
        assert!((thumb_len(100., 400., 200.) - 50.).abs() < 0.01);
        assert!((thumb_len(300., 400., 300.) - 225.).abs() < 0.01);
        // A quarter of a 100 px track would be 25 px, under the floor.
        assert_eq!(thumb_len(100., 400., 100.), f32::from(MIN_THUMB));
        // 10 000 px of content in a 100 px box would give a 1 px thumb.
        assert_eq!(thumb_len(100., 10_000., 100.), f32::from(MIN_THUMB));
        // …and the floor never exceeds the track.
        assert_eq!(thumb_len(10., 10_000., 10.), 10.);
    }

    /// A boundary records the run of registry entries its own paint added, and
    /// re-adds exactly that run — in order — on the frames it replays.
    ///
    /// Order is the whole of it: the router reads the registry as
    /// innermost-first, and that ordering is a property of WHERE in the paint
    /// walk each entry was pushed. Re-pushing the span at the point the
    /// subtree occupied is what keeps a nested box ahead of the box that
    /// contains it.
    #[test]
    fn a_recorded_span_replays_in_the_order_it_was_recorded() {
        // The walk so far: one outer container was registered before us.
        let mut chain = vec![10u32];
        let mark = chain.len();
        // …the subtree paints for real and registers two of its own…
        chain.extend([20, 21]);
        let span = span_since(&chain, mark);
        assert_eq!(span, vec![20, 21]);

        // …and the next frame it replays: nothing pushes, so the span goes
        // back at the same point, ahead of whatever the parent adds after it.
        let mut next = vec![10u32];
        let mark = next.len();
        assert!(span_since(&next, mark).is_empty(), "a replay contributes nothing itself");
        next.extend(span.iter().copied());
        next.push(11);
        assert_eq!(next, vec![10, 20, 21, 11]);
    }

    /// A mark past the end is the "the subtree registered nothing" case, and it
    /// has to answer an empty span rather than panic — a subtree with no scroll
    /// container in it takes this path on every single frame.
    #[test]
    fn a_span_with_nothing_in_it_is_empty_not_a_panic() {
        let chain: Vec<u32> = vec![1, 2];
        assert!(span_since(&chain, 2).is_empty());
        assert!(span_since(&chain, 9).is_empty());
        assert!(ChainSpan::default().is_empty());
        assert_eq!(ChainSpan::default().len(), 0);
    }

    /// `overscroll-behavior` is one question with one answer per axis.
    #[test]
    fn overscroll_chains_only_where_css_says_so() {
        let auto = Overscroll::CHAINING;
        assert!(auto.chains(Axis::Vertical));
        assert!(auto.chains(Axis::Horizontal));
        let contained = Overscroll { x_chains: true, y_chains: false };
        assert!(!contained.chains(Axis::Vertical));
        assert!(contained.chains(Axis::Horizontal));
    }

    /// The width animation is pure in `now`, so it can be stepped without a
    /// window — which is the property that makes the hover widen testable.
    #[test]
    fn the_thumb_widens_over_time_and_settles() {
        let t0 = Instant::now();
        let mut s = State::default();
        // The first frame adopts rather than animating in from zero.
        assert_eq!(animate_width(&mut s, 0, px(6.), t0), px(6.));
        // A new target starts a transition…
        let mid = animate_width(&mut s, 0, px(8.), t0 + EXPAND / 4);
        assert!(mid > px(6.) && mid < px(8.), "{mid:?} is between the two widths");
        // …which settles exactly on the target once the duration is up.
        assert_eq!(animate_width(&mut s, 0, px(8.), t0 + EXPAND * 2), px(8.));
    }

    /// The fade is the other pure transition: a bar that goes invisible does
    /// not blink out, and one that comes back does not fade in from where it
    /// was not.
    #[test]
    fn visibility_fades_from_wherever_it_was() {
        let t0 = Instant::now();
        let mut s = State { visible: true, fade_from: 0.0, fade_at: t0, ..State::default() };
        assert_eq!(sample_opacity(&s, t0), 0.0);
        assert!(sample_opacity(&s, t0 + FADE / 2) > 0.3);
        assert_eq!(sample_opacity(&s, t0 + FADE * 2), 1.0);
        // Interrupted halfway down, it resumes from the opacity it had.
        s.visible = false;
        s.fade_from = 0.5;
        s.fade_at = t0;
        assert_eq!(sample_opacity(&s, t0), 0.5);
        assert!(sample_opacity(&s, t0 + FADE / 2) < 0.5);
        assert_eq!(sample_opacity(&s, t0 + FADE * 2), 0.0);
    }

    /// `scrollbar-width: none` is not "do not scroll" — it is "do not draw".
    #[test]
    fn hiding_the_bar_keeps_the_style_otherwise_intact() {
        let base = ScrollbarStyle::from_text_color(gpui::black());
        assert_eq!(base.track_width, Some(TRACK_AUTO));
        assert_eq!(base.thin().track_width, Some(TRACK_THIN));
        assert_eq!(base.hidden().track_width, None);
        assert_eq!(base.hidden().thumb, base.thumb);
    }

    /// The default thumb takes its hue from the text, which is what makes it
    /// readable on a dark page and on a light one with no theme in between.
    #[test]
    fn the_default_thumb_follows_the_text_colour() {
        let dark_text = gpui::hsla(0., 0., 0.1, 1.0);
        let light_text = gpui::hsla(0., 0., 0.95, 1.0);
        let on_light = ScrollbarStyle::from_text_color(dark_text);
        let on_dark = ScrollbarStyle::from_text_color(light_text);
        assert!(on_light.thumb.l < 0.5, "dark text (light page) → dark thumb");
        assert!(on_dark.thumb.l > 0.5, "light text (dark page) → light thumb");
        // Both are translucent — an overlay scrollbar is never a solid slab.
        assert!(on_light.thumb.a < 0.5 && on_light.thumb_active.a > on_light.thumb.a);
        // …and the track starts fully transparent, which is what "reserves no
        // space and paints no gutter" looks like.
        assert_eq!(on_light.track.a, 0.0);
    }

    /// `scrollbar-color` is the author's answer, and it wins outright — the
    /// hover state does not invent a second colour they did not choose.
    #[test]
    fn scrollbar_color_replaces_both_thumb_states() {
        let thumb = gpui::hsla(0.5, 1., 0.5, 1.);
        let track = gpui::hsla(0.1, 1., 0.9, 1.);
        let s = ScrollbarStyle::from_text_color(gpui::black()).with_colors(thumb, track);
        assert_eq!(s.thumb, thumb);
        assert_eq!(s.thumb_active, thumb);
        assert_eq!(s.track, track);
    }
}
