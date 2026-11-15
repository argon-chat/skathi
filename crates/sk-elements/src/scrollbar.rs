//! Overlay scrollbars, and the wheel routing that makes `overscroll-behavior` work.
//!
//! Adapted from longbridge/gpui-kit `crates/base/src/scrollbar.rs` and `scrollable_mask.rs` at
//! commit `b6f098db81`, Copyright 2024–2026 Longbridge, licensed under the Apache License 2.0.
//!
//! The wheel is intercepted in the capture phase: gpui's own listener bubbles without stopping
//! propagation, so every ancestor scroll box would scroll at once. A gesture locks to the axis it
//! began on.

use std::cell::Cell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    fill, point, px, relative, size, App, Axis, Bounds, ContentMask, Element, ElementId,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, LayoutId, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Position, ScrollEvent, ScrollHandle, Style,
    Window, WindowControlArea,
};

const TRACK_AUTO: Pixels = px(14.);
const TRACK_THIN: Pixels = px(8.);
const INSET: Pixels = px(3.);
const MIN_THUMB: Pixels = px(28.);
const IDLE: Duration = Duration::from_millis(1200);
const FADE: Duration = Duration::from_millis(220);
const EXPAND: Duration = Duration::from_millis(120);
/// Drag updates are clamped to this, and to one pixel of movement.
const DRAG_FPS: u64 = 120;

/// `overscroll-behavior` for one axis: does a wheel this box cannot use reach the ancestors?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Overscroll {
    pub x_chains: bool,
    pub y_chains: bool,
}

impl Overscroll {
    pub const CHAINING: Self = Self {
        x_chains: true,
        y_chains: true,
    };

    fn chains(self, axis: Axis) -> bool {
        match axis {
            Axis::Horizontal => self.x_chains,
            Axis::Vertical => self.y_chains,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarStyle {
    /// `None` is `scrollbar-width: none`: the box still scrolls and routes the wheel, but draws nothing.
    pub track_width: Option<Pixels>,
    pub thumb: Hsla,
    pub thumb_active: Hsla,
    pub track: Hsla,
    pub track_active: Hsla,
    pub always_visible: bool,
}

impl ScrollbarStyle {
    /// The default look: the text colour at low alpha, so it contrasts on any background.
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

    pub fn with_colors(mut self, thumb: Hsla, track: Hsla) -> Self {
        self.thumb = thumb;
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

#[derive(Clone, Copy, Debug)]
struct State {
    hovered: Option<Axis>,
    dragging: Option<Axis>,
    grab: Pixels,
    last_offset: Point<Pixels>,
    last_scroll: Option<Instant>,
    last_drag_update: Instant,
    idle_timer: bool,
    lock: Option<(Axis, Instant)>,
    width: [Pixels; 2],
    width_at: Instant,
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

/// Silence of this length ends a wheel gesture and frees the axis lock.
const GESTURE_GAP: Duration = Duration::from_millis(180);

#[derive(Clone)]
struct ChainEntry {
    hitbox: gpui::HitboxId,
    handle: ScrollHandle,
    ancestry: GlobalElementId,
    overscroll: Overscroll,
    scrolls_x: bool,
    scrolls_y: bool,
}

thread_local! {
    /// Scroll containers painted last frame, in paint order. Deferred children can paint after their
    /// ancestors, so nesting comes from element-id ancestry, not this order.
    static CHAIN: std::cell::RefCell<Vec<ChainEntry>> = const { std::cell::RefCell::new(Vec::new()) };

    /// Scroll containers under the pointer with the offsets the current wheel event found. See [`undo_wheel_scroll`].
    static BEFORE_WHEEL: std::cell::RefCell<Vec<(gpui::HitboxId, ScrollHandle, Point<Pixels>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Puts every scroll container under the pointer back where the current wheel event found it, and
/// says whether one had moved. For a cancelled `wheel`: gpui's listeners have already scrolled, so
/// the embedder calls this and stops propagation. Only meaningful during that event's dispatch.
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
    /// Per-scrollbar state, keyed by the bar's `ElementId`. Not gpui element state: `Window::use_state`
    /// is keyed by the whole element-id path, which changes when a retained-render boundary is
    /// re-offered with a new entity, and that reset an in-progress drag.
    static STATES: std::cell::RefCell<HashMap<u64, (u64, Rc<Cell<State>>)>> =
        std::cell::RefCell::new(HashMap::new());
    static FRAME: Cell<u64> = const { Cell::new(0) };
}

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
        let slot = m
            .entry(key)
            .or_insert_with(|| (frame, Rc::new(Cell::new(State::default()))));
        slot.0 = frame;
        slot.1.clone()
    })
}

/// Builds a scrollbar may go unseen before its state is dropped; large so a `<KeepAlive>`d page keeps its.
const STATE_TTL: u64 = 4096;

pub fn reset_frame() {
    CHAIN.with(|c| c.borrow_mut().clear());
    let frame = FRAME.with(|c| {
        let n = c.get().wrapping_add(1);
        c.set(n);
        n
    });
    if frame % STATE_TTL == 0 {
        STATES.with(|m| {
            m.borrow_mut()
                .retain(|_, (seen, _)| frame.saturating_sub(*seen) < STATE_TTL)
        });
    }
}

/// A contiguous run of registry entries a retained-render boundary added while painting for real.
/// A replayed subtree does not paint, so without re-registering its span the wheel router loses its
/// scroll containers. Replay is safe because `HitboxId`s are never reused within a window. Record
/// with [`chain_mark`] + [`chain_since`], replay with [`chain_replay`].
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
        f.debug_struct("ChainSpan")
            .field("entries", &self.0.len())
            .finish()
    }
}

/// `mark` / `since` over a `Vec`, generic so it can be tested without gpui.
fn span_since<T: Clone>(v: &[T], mark: usize) -> Vec<T> {
    v.get(mark..).map(<[T]>::to_vec).unwrap_or_default()
}

pub fn chain_mark() -> usize {
    CHAIN.with(|c| c.borrow().len())
}

pub fn chain_since(mark: usize) -> ChainSpan {
    CHAIN.with(|c| ChainSpan(span_since(&c.borrow(), mark)))
}

pub fn chain_replay(span: &ChainSpan) {
    if span.0.is_empty() {
        return;
    }
    CHAIN.with(|c| c.borrow_mut().extend(span.0.iter().cloned()));
}

pub fn chain_count() -> usize {
    chain_mark()
}

fn container_path(id: &GlobalElementId) -> &[ElementId] {
    match id.split_last() {
        Some((_, ancestors)) => ancestors,
        None => &[],
    }
}

fn chain_position<'a, Id: PartialEq + 'a>(
    own: &'a [Id],
    entries: impl IntoIterator<Item = (&'a [Id], bool, bool)>,
) -> (bool, bool) {
    let mut outer_moves = false;
    for (path, moves, chains) in entries {
        if path.len() > own.len() && path.starts_with(own) && (moves || !chains) {
            return (false, false);
        }
        if own.len() > path.len() && own.starts_with(path) && moves {
            outer_moves = true;
        }
    }
    (true, outer_moves)
}

fn effective_delta(axis: Axis, raw: Point<Pixels>, axes: (bool, bool)) -> (Axis, Pixels) {
    match axis {
        Axis::Vertical if !axes.1 && axes.0 => (Axis::Horizontal, raw.y),
        Axis::Vertical => (Axis::Vertical, raw.y),
        Axis::Horizontal if !axes.0 && axes.1 => (Axis::Vertical, raw.x),
        Axis::Horizontal => (Axis::Horizontal, raw.x),
    }
}

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

/// An overlay scrollbar for one `overflow: auto | scroll` box, plus its wheel routing; paint it as
/// the container's last child.
pub struct Scrollbar {
    id: ElementId,
    handle: ScrollHandle,
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

    /// Dispatches the DOM `wheel` event for a gesture this element takes over; `true` is `preventDefault()`.
    pub fn on_wheel(
        mut self,
        f: impl Fn(Point<Pixels>, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.on_wheel = Some(Rc::new(f));
        self
    }

    pub fn on_scrolled(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_scrolled = Some(Rc::new(f));
        self
    }

    pub fn axes(mut self, x: bool, y: bool) -> Self {
        self.axes = (x, y);
        self
    }

    pub fn overscroll(mut self, overscroll: Overscroll) -> Self {
        self.overscroll = overscroll;
        self
    }

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

struct AxisPaint {
    axis: Axis,
    track: Bounds<Pixels>,
    thumb_hit: Bounds<Pixels>,
    thumb_fill: Bounds<Pixels>,
    hitbox: Hitbox,
    max_offset: Pixels,
    thumb_travel: Pixels,
}

pub struct Prepaint {
    viewport: Option<Hitbox>,
    axes: Vec<AxisPaint>,
    state: Rc<Cell<State>>,
    opacity: f32,
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
        // Absolute so it is not a flex item of the box it overlays; `prepaint` reads the viewport off the handle.
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

        let viewport = self.handle.bounds();
        if viewport.size.width <= px(0.) || viewport.size.height <= px(0.) {
            return Prepaint {
                viewport: None,
                axes: Vec::new(),
                state,
                opacity: 0.0,
            };
        }

        let mut inner = state.get();
        let offset = self.handle.offset();
        if offset != inner.last_offset {
            inner.last_offset = offset;
            inner.last_scroll = Some(now);
        }

        // The idle timer is what hides the bar: a stopped scroll produces no more frames.
        let held = inner
            .last_scroll
            .is_some_and(|t| now.saturating_duration_since(t) < IDLE);
        let want = self.style.always_visible
            || held
            || inner.hovered.is_some()
            || inner.dragging.is_some();
        if want != inner.visible {
            inner.fade_from = sample_opacity(&inner, now);
            inner.fade_at = now;
            inner.visible = want;
        }
        let opacity = sample_opacity(&inner, now);
        if (opacity - if inner.visible { 1.0 } else { 0.0 }).abs() > f32::EPSILON {
            window.request_animation_frame();
        }
        if want && !self.style.always_visible && inner.hovered.is_none() && inner.dragging.is_none()
        {
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

        // Wheel box over the whole viewport; tagged passive (`gpui::VN_TAG_PASSIVE`) so embedder hit tests look through it.
        let hitbox = window.with_content_mask(Some(ContentMask::rect(viewport)), |window| {
            let tag = window.set_hitbox_tag(0);
            window.set_hitbox_tag(if tag == 0 {
                0
            } else {
                tag | gpui::VN_TAG_PASSIVE
            });
            let hitbox = window.insert_hitbox(viewport, HitboxBehavior::Normal);
            window.set_hitbox_tag(tag);
            hitbox
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
                if max_offset <= px(0.5) {
                    continue;
                }
                let margin_end = if both && !vertical { width } else { px(0.) };
                let track = if vertical {
                    Bounds {
                        origin: point(
                            viewport.origin.x + viewport.size.width - width,
                            viewport.origin.y,
                        ),
                        size: size(width, viewport.size.height),
                    }
                } else {
                    Bounds {
                        origin: point(
                            viewport.origin.x,
                            viewport.origin.y + viewport.size.height - width,
                        ),
                        size: size(viewport.size.width, width),
                    }
                };

                let track_len = if vertical {
                    track.size.height
                } else {
                    track.size.width
                } - margin_end;
                let content = viewport_len + max_offset;
                let thumb_len = (track_len * (viewport_len / content))
                    .max(MIN_THUMB)
                    .min(track_len);
                let travel = track_len - thumb_len;
                let scrolled = if vertical { -offset.y } else { -offset.x };
                let start = (travel * (scrolled / max_offset).clamp(0., 1.)).max(px(0.));

                let hovered =
                    state.get().hovered == Some(axis) || state.get().dragging == Some(axis);
                let target = if hovered {
                    width - INSET * 2.
                } else {
                    (width - INSET * 2.) * 0.66
                };
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

        Prepaint {
            viewport: Some(hitbox),
            axes,
            state,
            opacity,
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(viewport) = prepaint.viewport.clone() else {
            return;
        };
        let view = window.current_view();
        let state = prepaint.state.clone();
        let opacity = prepaint.opacity;
        let bounds = viewport.bounds;

        // Scroll chain: gpui's wheel listener moves every ancestor too, so take over only when this is
        // the nearest box to use the wheel AND an ancestor could also move.
        let ancestry = global_id.cloned().unwrap_or_default();
        CHAIN.with(|c| {
            c.borrow_mut().push(ChainEntry {
                hitbox: viewport.id,
                handle: self.handle.clone(),
                ancestry: ancestry.clone(),
                overscroll: self.overscroll,
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

                // Axis lock is per gesture so a diagonal swipe cannot change owners.
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

                let gesture_axis = axis;
                let (axis, delta) = effective_delta(gesture_axis, raw, (wants_x, wants_y));

                let (can_claim, outer_moves) = CHAIN.with(|c| {
                    let chain = c.borrow();
                    chain_position(
                        container_path(&ancestry),
                        chain
                            .iter()
                            .filter(|entry| entry.hitbox.should_handle_scroll(window))
                            .map(|entry| {
                                let (entry_axis, entry_delta) = effective_delta(
                                    gesture_axis,
                                    raw,
                                    (entry.scrolls_x, entry.scrolls_y),
                                );
                                let usable = match entry_axis {
                                    Axis::Vertical => entry.scrolls_y,
                                    Axis::Horizontal => entry.scrolls_x,
                                };
                                (
                                    container_path(&entry.ancestry),
                                    usable && can_move(&entry.handle, entry_axis, entry_delta),
                                    entry.overscroll.chains(entry_axis),
                                )
                            }),
                    )
                });
                if !can_claim || !outer_moves {
                    return;
                }

                let usable = match axis {
                    Axis::Vertical => wants_y,
                    Axis::Horizontal => wants_x,
                };
                // Take over only when that changes something: if this box is at its edge and the
                // axis chains, gpui's listener already does the right thing. Dispatching `wheel`
                // here too would fire `@wheel` twice (once here, once from the renderer's `on_scroll`).
                let will_stop =
                    (usable && can_move(&handle, axis, delta)) || !overscroll.chains(axis);
                if !will_stop {
                    return;
                }

                // DOM `wheel` first: it is cancelable and fires before scrolling.
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
                if moved || prevented || !overscroll.chains(axis) {
                    cx.stop_propagation();
                }
            });
        }

        // Release is handled per axis, so a drag whose track vanished (content shrank) would stay `dragging`.
        let mut s = state.get();
        if s.dragging
            .is_some_and(|axis| !prepaint.axes.iter().any(|ax| ax.axis == axis))
        {
            s.dragging = None;
            state.set(s);
        }

        if prepaint.axes.is_empty() || opacity <= 0.001 {
            return;
        }

        window.with_content_mask(Some(ContentMask::rect(bounds)), |window| {
            for ax in prepaint.axes.iter() {
                let axis = ax.axis;
                // A `window-drag: move` titlebar reaching the viewport edge would make Win32 start a
                // window drag before the press reaches us (`WM_NCHITTEST` is answered first); `NoDrag`
                // opts back out. Paint-phase only.
                window.insert_window_control_hitbox(WindowControlArea::NoDrag, ax.hitbox.clone());
                let hovered =
                    state.get().hovered == Some(axis) || state.get().dragging == Some(axis);
                let track_bg = if hovered {
                    self.style.track_active
                } else {
                    self.style.track
                };
                let thumb_bg = if hovered {
                    self.style.thumb_active
                } else {
                    self.style.thumb
                };
                let radius = (ax.thumb_fill.size.width.min(ax.thumb_fill.size.height)) / 2.;

                window.paint_layer(bounds, |window| {
                    window.paint_quad(fill(ax.track, faded(track_bg, opacity)));
                    window.paint_quad(
                        fill(ax.thumb_fill, faded(thumb_bg, opacity)).corner_radii(radius),
                    );
                });

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
                        if !phase.bubble()
                            || !hitbox.is_hovered(window)
                            || !track.contains(&event.position)
                        {
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
                            let half = if vertical {
                                thumb_hit.size.height
                            } else {
                                thumb_hit.size.width
                            } / 2.;
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
                            if vertical {
                                o.y = next
                            } else {
                                o.x = next
                            }
                            handle.set_offset(o);
                            s.last_offset = o;
                            jumped = true;
                        }
                        s.dragging = Some(axis);
                        s.last_scroll = Some(Instant::now());
                        state.set(s);
                        if jumped {
                            if let Some(f) = &on_scrolled {
                                f(window, cx);
                            }
                        }
                        cx.notify(view);
                    });
                }

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
                        // Our own `dragging`: synthesized moves (devtools, tablet) carry no `pressed_button`.
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
                            let current = if vertical {
                                handle.offset().y
                            } else {
                                handle.offset().x
                            };
                            if (next - current).abs() > px(1.)
                                && Instant::now().saturating_duration_since(s.last_drag_update)
                                    >= throttle
                            {
                                let mut o = handle.offset();
                                if vertical {
                                    o.y = next
                                } else {
                                    o.x = next
                                }
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

                {
                    let handle = self.handle.clone();
                    let state = state.clone();
                    let track = ax.track;
                    let max_offset = ax.max_offset;
                    let travel = ax.thumb_travel;
                    let on_scrolled = self.on_scrolled.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if !phase.bubble() {
                            return;
                        }
                        let mut s = state.get();
                        if s.dragging == Some(axis) {
                            // Throttled moves may have dropped the last one, so apply the release position.
                            let vertical = axis == Axis::Vertical;
                            let along = if vertical {
                                event.position.y - track.origin.y
                            } else {
                                event.position.x - track.origin.x
                            };
                            let mut o = handle.offset();
                            let current = if vertical { o.y } else { o.x };
                            let next = if travel > px(0.) {
                                -max_offset * ((along - s.grab) / travel).clamp(0., 1.)
                            } else {
                                current
                            };
                            let moved = (next - current).abs() > px(0.5);
                            if moved {
                                if vertical {
                                    o.y = next
                                } else {
                                    o.x = next
                                }
                                handle.set_offset(o);
                                s.last_offset = o;
                            }
                            s.dragging = None;
                            s.last_scroll = Some(Instant::now());
                            state.set(s);
                            if moved {
                                if let Some(f) = &on_scrolled {
                                    f(window, cx);
                                }
                            }
                            cx.notify(view);
                        }
                    });
                }
            }
        });
        let _ = cx;
    }
}

/// gpui's wheel delta has the opposite sign of the DOM's.
fn dom_delta(delta: Point<Pixels>) -> Point<Pixels> {
    point(-delta.x, -delta.y)
}

fn faded(color: Hsla, opacity: f32) -> Hsla {
    Hsla {
        a: color.a * opacity,
        ..color
    }
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

fn animate_width(state: &mut State, idx: usize, target: Pixels, now: Instant) -> Pixels {
    let from = state.width[idx];
    if from == Pixels::ZERO {
        state.width[idx] = target;
        state.width_at = now;
        return target;
    }
    if (from - target).abs() < px(0.1) {
        state.width[idx] = target;
        return target;
    }
    let t = now.saturating_duration_since(state.width_at).as_secs_f32() / EXPAND.as_secs_f32();
    let next = if t >= 1.0 {
        target
    } else {
        from + (target - from) * ease_out(t)
    };
    if t >= 1.0 || (next - from).abs() > px(0.05) {
        state.width[idx] = next;
        state.width_at = now;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deferred_child_owns_the_wheel_in_every_paint_order() {
        let root = [1u32];
        let parent = [1, 2];
        let child = [1, 2, 3];
        let entries: [(&[u32], bool, bool); 3] = [
            (&root, true, true),
            (&parent, true, true),
            (&child, true, true),
        ];
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let painted = order.map(|index| entries[index]);
            assert_eq!(chain_position(&child, painted), (true, true));
            assert_eq!(chain_position(&parent, painted), (false, false));
            assert_eq!(chain_position(&root, painted), (false, false));
        }
    }

    #[test]
    fn an_auto_child_at_its_edge_yields_only_to_the_nearest_movable_ancestor() {
        let root = [1u32];
        let parent = [1, 2];
        let child = [1, 2, 3];
        for child_chains in [true, false] {
            let entries: [(&[u32], bool, bool); 3] = [
                (&root, true, true),
                (&parent, true, true),
                (&child, false, child_chains),
            ];
            for order in [
                [0, 1, 2],
                [0, 2, 1],
                [1, 0, 2],
                [1, 2, 0],
                [2, 0, 1],
                [2, 1, 0],
            ] {
                let painted = order.map(|index| entries[index]);
                assert_eq!(chain_position(&root, painted), (false, false));
                assert_eq!(
                    chain_position(&parent, painted),
                    if child_chains {
                        (true, true)
                    } else {
                        (false, false)
                    }
                );
                assert_eq!(chain_position(&child, painted), (true, true));
            }
        }
    }

    #[test]
    fn chain_membership_uses_ancestry_instead_of_path_depth_or_paint_position() {
        let root = [1u32];
        let child = [1, 2, 3];
        let sibling = [1, 4, 5, 6];
        let entries: [(&[u32], bool, bool); 3] = [
            (&sibling, true, false),
            (&root, false, true),
            (&child, true, true),
        ];
        assert_eq!(chain_position(&child, entries), (true, false));
        let ancestors: [(&[u32], bool, bool); 2] = [(&root, true, true), (&child, false, true)];
        assert_eq!(chain_position(&child, ancestors), (true, true));
    }

    #[test]
    fn replaying_a_span_after_its_ancestor_keeps_the_child_as_wheel_owner() {
        let root = [1u32];
        let child = [1, 2, 3];
        let recorded: Vec<(&[u32], bool, bool)> = vec![(&root, true, true), (&child, true, true)];
        let span = span_since(&recorded, 1);
        let mut replayed: Vec<(&[u32], bool, bool)> = vec![(&root, true, true)];
        replayed.extend(span);
        assert_eq!(
            chain_position(&root, replayed.iter().copied()),
            (false, false)
        );
        assert_eq!(
            chain_position(&child, replayed.iter().copied()),
            (true, true)
        );
    }

    #[test]
    fn each_chain_candidate_maps_the_gesture_to_its_own_scroll_axis() {
        let raw = point(px(0.), px(-80.));
        assert_eq!(
            effective_delta(Axis::Vertical, raw, (false, true)),
            (Axis::Vertical, px(-80.))
        );
        assert_eq!(
            effective_delta(Axis::Vertical, raw, (true, false)),
            (Axis::Horizontal, px(-80.))
        );
        assert_eq!(
            effective_delta(Axis::Vertical, raw, (true, true)),
            (Axis::Vertical, px(-80.))
        );
    }

    #[test]
    fn a_recorded_span_replays_in_the_order_it_was_recorded() {
        let mut chain = vec![10u32];
        let mark = chain.len();
        chain.extend([20, 21]);
        let span = span_since(&chain, mark);
        assert_eq!(span, vec![20, 21]);

        let mut next = vec![10u32];
        let mark = next.len();
        assert!(
            span_since(&next, mark).is_empty(),
            "a replay contributes nothing itself"
        );
        next.extend(span.iter().copied());
        next.push(11);
        assert_eq!(next, vec![10, 20, 21, 11]);
    }

    #[test]
    fn a_span_with_nothing_in_it_is_empty_not_a_panic() {
        let chain: Vec<u32> = vec![1, 2];
        assert!(span_since(&chain, 2).is_empty());
        assert!(span_since(&chain, 9).is_empty());
    }

    #[test]
    fn the_thumb_widens_over_time_and_settles() {
        let t0 = Instant::now();
        let mut s = State::default();
        assert_eq!(animate_width(&mut s, 0, px(6.), t0), px(6.));
        let mid = animate_width(&mut s, 0, px(8.), t0 + EXPAND / 4);
        assert!(
            mid > px(6.) && mid < px(8.),
            "{mid:?} is between the two widths"
        );
        assert_eq!(animate_width(&mut s, 0, px(8.), t0 + EXPAND * 2), px(8.));
    }

    #[test]
    fn visibility_fades_from_wherever_it_was() {
        let t0 = Instant::now();
        let mut s = State {
            visible: true,
            fade_from: 0.0,
            fade_at: t0,
            ..State::default()
        };
        assert_eq!(sample_opacity(&s, t0), 0.0);
        assert!(sample_opacity(&s, t0 + FADE / 2) > 0.3);
        assert_eq!(sample_opacity(&s, t0 + FADE * 2), 1.0);
        s.visible = false;
        s.fade_from = 0.5;
        s.fade_at = t0;
        assert_eq!(sample_opacity(&s, t0), 0.5);
        assert!(sample_opacity(&s, t0 + FADE / 2) < 0.5);
        assert_eq!(sample_opacity(&s, t0 + FADE * 2), 0.0);
    }

    #[test]
    fn the_default_thumb_follows_the_text_colour() {
        let dark_text = gpui::hsla(0., 0., 0.1, 1.0);
        let light_text = gpui::hsla(0., 0., 0.95, 1.0);
        let on_light = ScrollbarStyle::from_text_color(dark_text);
        let on_dark = ScrollbarStyle::from_text_color(light_text);
        assert!(
            on_light.thumb.l < 0.5,
            "dark text (light page) → dark thumb"
        );
        assert!(
            on_dark.thumb.l > 0.5,
            "light text (dark page) → light thumb"
        );
        assert!(on_light.thumb.a < 0.5 && on_light.thumb_active.a > on_light.thumb.a);
        assert_eq!(on_light.track.a, 0.0);
    }
}
