//! `<lottie>` — the element layer over the `vn-lottie` crate.
//!
//! This module owns everything that outlives a frame: the per-node
//! [`LottieState`] side map, the `src` → bytes load, the prop→transport sync,
//! the once-per-frame tick and the event dispatch. The gpui element itself is
//! built in [`crate::render`] (`build_lottie`), which is where the cascade,
//! the focus handles and the listener wiring already live.
//!
//! `crates/vn-lottie/CONTRACT.md` is the spec this implements; §"Element-layer
//! decisions" there records the three places the element layer deliberately
//! overrides the crate.
//!
//! # The three owner-level decisions, in one place
//!
//! 1. **`autoplay` and `loop` default to `false`.** The crate defaults both to
//!    lottie-web's `loadAnimation()` values (`true`/`true`); `<lottie>` is
//!    markup, and the markup-facing precedent is the `<lottie-player>` web
//!    component, which defaults both to `false`. Overridden here rather than in
//!    the crate so a Rust embedder driving `LottieState` directly still gets
//!    lottie-web's semantics.
//! 2. **`loopComplete` never fires on the boundary that completes playback.**
//!    lottie-web fires `complete` alone there. The crate's `Advance` reports
//!    both, so the last `LoopComplete` of a completing tick is filtered out at
//!    the dispatch site (see [`dispatch_events`]).
//! 3. **`:loop="n"` maps to `LoopMode::Count(n + 1)`.** lottie-web's numeric
//!    `loop` counts *repeats* — `loop: 2` plays three times — while the crate's
//!    `Count(n)` plays exactly `n` times. The `+ 1` is what makes `:loop="2"`
//!    behave like the browser; combined with (2) the event stream is then
//!    lottie-web's exactly: `loopComplete` ×n, then `complete`.
//!
//! # Detached nodes keep their clock running
//!
//! A `<KeepAlive>`-cached page is moved into a detached container, not
//! unmounted, so its nodes stay allocated and their entries stay in this map.
//! They keep advancing while off-screen — which is what a browser does with a
//! `<lottie-player>` in a detached subtree (its rAF loop is never cancelled) —
//! so coming back shows the animation where it *would* be, not where it was
//! left. Nothing is painted meanwhile, and the raster cache is only refreshed
//! when the element is actually built, so the cost is the transport arithmetic.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, RenderImage, Window};
use vn_lottie::{Advance, Direction, LoopMode, LottieEvent, LottieState, PlayMode};

use crate::tree::{self, Tree};
use crate::EventPayload;

/// One `<lottie>` node's persistent state.
struct Entry {
    /// The raw `src` string the state was built from — the (re)load key.
    src: String,
    /// `None` when the source could not be loaded at all. Remembered rather
    /// than retried, so a broken `src` costs one advisory, not one per frame.
    state: Option<LottieState>,
    /// Last `autoplay` value seen, so play/pause is edge-triggered off the
    /// *prop* (a `v-bind` that never changes must not restart a paused
    /// animation every frame).
    autoplay: bool,
    /// **The window that painted this node last** — recorded at creation, not
    /// at drop (docs/MULTIWINDOW.md §5.1). `Window::drop_image` is per-window:
    /// the sprite atlas belongs to the window, so a raster has to go back to
    /// the window that put it there. By the time `gc` runs the node is already
    /// out of the tree and the ancestor walk that would answer "which window"
    /// has nothing left to walk — which is why it is stored rather than
    /// derived.
    window: crate::windows::WinKey,
}

thread_local! {
    static LOTTIE_STATES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
    /// Wall clock for the tick. `None` until the first frame — the pump's own
    /// cadence is what defines `dt`, exactly as it does for CSS transitions.
    static LAST_TICK: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Rasters whose node was GC'd. `gc_node_state` has no `Window`, and dropping a
/// `RenderImage` without `Window::drop_image` leaks its atlas tiles
/// (CONTRACT.md §3) — so they wait for the next frame that does.
///
/// **Queued against the window that created the tile**
/// (`WindowState::pending_drops`), because the atlas is the window's. gpui's
/// `App::drop_image(image, Some(window))` sweeps every *other* window's atlas
/// as a courtesy, but the one window it cannot reach through `App::windows` is
/// precisely the one whose update we are standing inside — so the queue has to
/// be keyed by the window that will drain it, or a pop-out's tiles are freed
/// from every atlas except the one holding them.
fn queue_drops(win: crate::windows::WinKey, images: impl IntoIterator<Item = Arc<RenderImage>>) {
    crate::windows::of(win, |w| w.pending_drops.extend(images));
}

/// A tick longer than this is treated as a stall (debugger, minimised window,
/// a long HMR patch) and clamped, so an animation does not teleport. Browsers
/// do the same by simply not running rAF while hidden.
const MAX_DT: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------------------
// props
// ---------------------------------------------------------------------------

/// `loop` — browser boolean-attribute grammar, plus lottie-web's numeric form.
///
/// Absent is **`LoopMode::None`**: the `<lottie-player>` default, not
/// lottie-web's (decision 1 in the module docs). `:loop="n"` gets the `+ 1`
/// that turns the crate's "play n times" into lottie-web's "repeat n times"
/// (decision 3).
fn loop_mode_from(raw: Option<&str>) -> LoopMode {
    match raw.map(str::trim) {
        None | Some("false") => LoopMode::None,
        Some("") | Some("true") => LoopMode::Loop,
        Some(v) => match v.parse::<f64>() {
            Ok(n) if n >= 1.0 => LoopMode::Count(n as u32 + 1),
            // `loop="0"` is falsy in lottie-web, like `loop: 0`.
            Ok(_) => LoopMode::None,
            // Any other non-empty attribute value is truthy in HTML.
            Err(_) => LoopMode::Loop,
        },
    }
}

/// The browser's boolean-attribute rule: present is true unless it spells
/// `"false"` (which is what `:attr="false"` serialises to through `patchProp`).
fn bool_attr(raw: Option<&str>) -> bool {
    raw.is_some_and(|v| v.trim() != "false")
}

/// `segment="in,out"`. An array binding (`:segment="[10, 40]"`) cannot reach
/// here — `patchProp` drops non-primitive props with its own advisory — so the
/// string spelling is the supported one, and it is what gets documented.
fn segment_from(raw: Option<&str>) -> Option<(f32, f32)> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let (a, b) = match raw.split_once(',') {
        Some(pair) => pair,
        None => raw.split_once(char::is_whitespace)?,
    };
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Props lottie-web / `<lottie-player>` have and this renderer does not.
const UNSUPPORTED_PROPS: &[(&str, &str)] = &[
    ("renderer", "rendering is always the software rasteriser; there is no svg/canvas/html renderer to choose"),
    ("background", "use the CSS `background-color` / `background-image` on the element — the raster is transparent, so it shows through"),
    ("controls", "no built-in transport UI; build one in Vue and drive `autoplay` / `speed` / `direction`"),
    ("hover", "no hover-to-play; bind `autoplay` to a ref and flip it from @mouseenter / @mouseleave"),
    ("intermission", "no pause between loops"),
    ("count", "use `loop=\"<n>\"`, which is lottie-web's spelling of the same thing"),
    ("preserveaspectratio", "use the CSS `object-fit` property (or the `objectfit` attribute)"),
    ("webworkers", "there is no worker to move rasterisation to"),
    ("subframe", "subframe rendering is always on — the playhead is continuous and is never snapped to source frames"),
];

fn warn_unsupported_props(t: &Tree, id: u32) {
    for (name, alternative) in UNSUPPORTED_PROPS {
        if t.prop(id, name).is_some() {
            crate::render::warn_compat(
                &format!("lottie-prop:{name}"),
                &format!("<lottie {name}=…> is not supported — {alternative}"),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// lifecycle
// ---------------------------------------------------------------------------

/// Creates the state if `src` changed (or there is none yet), then pushes every
/// prop into the transport. Returns the animation's intrinsic size, which is
/// what the box falls back to when CSS declares none — `None` when nothing is
/// loaded, i.e. the element paints nothing at all.
///
/// Called from the render walk, once per node per frame. Everything expensive
/// (file read, JSON parse, `Renderer::prepare`) is behind the `src` comparison;
/// the rest is a handful of setters the crate itself dedupes.
pub(crate) fn sync(t: &Tree, id: u32, win: crate::windows::WinKey) -> Option<(u32, u32)> {
    warn_unsupported_props(t, id);
    let src = t.prop(id, "src").unwrap_or("").to_string();
    let autoplay = bool_attr(t.prop(id, "autoplay"));

    LOTTIE_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let fresh = match map.get(&id) {
            Some(entry) => entry.src != src,
            None => true,
        };
        if fresh {
            // The old state's rasters have to go back to gpui, and this is not
            // the place with a `Window` — queue them like the GC does.
            if let Some(mut old) = map.remove(&id) {
                if let Some(state) = old.state.as_mut() {
                    queue_drops(old.window, state.take_all_images());
                }
            }
            map.insert(id, Entry { src: src.clone(), state: load(&src), autoplay, window: win });
        }

        let entry = map.get_mut(&id)?;
        // Re-stamped every frame: a node can move between windows over its life
        // (pop out, come back), and the tiles it holds belong to whichever
        // window painted them last.
        entry.window = win;
        let state = entry.state.as_mut()?;

        // --- transport, every frame (the crate no-ops on unchanged values) ---
        state.set_speed(
            t.prop(id, "speed")
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(1.0),
        );
        state.set_direction(match t.prop(id, "direction").map(str::trim) {
            Some("-1") | Some("reverse") => Direction::Reverse,
            _ => Direction::Forward,
        });
        state.set_play_mode(match t.prop(id, "mode").map(str::trim) {
            Some("bounce") => PlayMode::Bounce,
            _ => PlayMode::Normal,
        });
        state.set_segment(segment_from(t.prop(id, "segment")));

        // `set_loop_mode` clears `finished` for any looping mode, so flipping
        // `:loop` on a completed animation makes it run again — lottie-web's
        // `setLoop` does the same.
        state.set_loop_mode(loop_mode_from(t.prop(id, "loop")));

        // `autoplay` is EDGE-triggered off the prop. On the load frame it
        // decides whether the animation starts at all (the crate loads
        // playing); afterwards a change is a play/pause command, which is what
        // makes `:autoplay="isPlaying"` work as the transport control markup
        // needs it to be. `<lottie-player>` only reads `autoplay` at load and
        // exposes `.play()` / `.pause()` instead — we have no element methods,
        // so this is a deliberate, documented superset.
        if fresh {
            if autoplay {
                state.play();
            } else {
                state.pause();
            }
        } else if autoplay != entry.autoplay {
            if autoplay {
                state.play();
            } else {
                state.pause();
            }
        }
        entry.autoplay = autoplay;

        let meta = state.meta();
        Some((meta.width, meta.height))
    })
}

/// Loads one `src`, routing every failure through the compat channel. `None`
/// means "nothing to paint" — a broken `<lottie>` behaves like a broken `<img>`:
/// an advisory, an empty box, never a panic.
fn load(src: &str) -> Option<LottieState> {
    if src.trim().is_empty() {
        // An absent `src` is not an error worth shouting about (a `v-if`'d or
        // not-yet-resolved binding hits this on its first frame).
        return None;
    }
    let bytes = match crate::img::read_bytes("lottie", src) {
        Ok(bytes) => bytes,
        Err(msg) => {
            crate::render::warn_compat(&format!("lottie-src:{src}"), &msg);
            return None;
        }
    };
    match LottieState::load(&bytes) {
        Ok(state) => {
            // A *successful* load can still have had to rewrite the document.
            for advisory in state.advisories() {
                crate::render::warn_compat(
                    &format!("lottie-load:{src}:{advisory}"),
                    &format!("<lottie src=\"{src}\">: {advisory}"),
                );
            }
            Some(state)
        }
        Err(error) => {
            crate::render::warn_compat(
                &format!("lottie-load:{src}"),
                &format!("<lottie src=\"{src}\"> could not be loaded — {error}"),
            );
            for advisory in error.advisories() {
                crate::render::warn_compat(
                    &format!("lottie-unsupported:{src}:{advisory}"),
                    &format!("<lottie src=\"{src}\">: unsupported feature — {advisory}"),
                );
            }
            None
        }
    }
}

/// Runs `f` with one node's live state, if it has one.
pub(crate) fn with_state<R>(id: u32, f: impl FnOnce(&mut LottieState) -> R) -> Option<R> {
    LOTTIE_STATES.with(|m| m.borrow_mut().get_mut(&id)?.state.as_mut().map(f))
}

/// Drops states for nodes that no longer exist, handing their rasters back
/// through [`PENDING_DROPS`] — see CONTRACT.md §3: a plain `retain` here leaks
/// one atlas tile per cached frame.
pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    LOTTIE_STATES.with(|m| {
        m.borrow_mut().retain(|&id, entry| {
            if live(id) {
                return true;
            }
            if let Some(state) = entry.state.as_mut() {
                queue_drops(entry.window, state.take_all_images());
            }
            false
        })
    });
}

// ---------------------------------------------------------------------------
// the frame pump
// ---------------------------------------------------------------------------

/// True while any `<lottie>` still needs frames — folded into the host's
/// `animating()` OR-chain (CONTRACT.md §4).
pub fn any_playing() -> bool {
    LOTTIE_STATES
        .with(|m| vn_lottie::any_playing(m.borrow().values().filter_map(|e| e.state.as_ref())))
}

/// Advances every `<lottie>` by the wall-clock delta since the previous frame
/// and dispatches the events that produced. Returns `true` when something moved
/// and the window must repaint.
///
/// Runs from the host's frame pump, **before** the draw that builds the
/// elements, so the raster a node paints is the one this tick just moved to.
pub fn tick(window: &mut Window, cx: &mut App) -> bool {
    drain_pending_drops(window, cx);

    let now = Instant::now();
    let dt = LAST_TICK
        .with(|t| t.replace(Some(now)))
        .map(|prev| (now - prev).min(MAX_DT))
        .unwrap_or_default();
    if dt.is_zero() {
        return false;
    }

    // Collect first, dispatch after: a JS handler can mutate the tree (and
    // `flush_changes` re-enters `gc`), so the map must not be borrowed while
    // any of this reaches JavaScript.
    let ticks: Vec<(u32, Advance)> = LOTTIE_STATES.with(|m| {
        m.borrow_mut()
            .iter_mut()
            .filter_map(|(&id, entry)| {
                let advance = entry.state.as_mut()?.advance(dt);
                let interesting = advance.needs_repaint
                    || advance.completed
                    || advance.loops_completed > 0
                    || advance.entered_frame.is_some();
                interesting.then_some((id, advance))
            })
            .collect()
    });

    let moved = !ticks.is_empty();
    for (id, advance) in ticks {
        dispatch_events(id, &advance, window, cx);
    }
    moved
}

/// [`drain_pending_drops`] for a window that is not the one the pump runs
/// with — a pop-out draining its own atlas from its own draw.
pub(crate) fn drain_pending_drops_for(window: &mut Window, cx: &mut App) {
    drain_pending_drops(window, cx);
}

fn drain_pending_drops(window: &mut Window, cx: &mut App) {
    let images = crate::windows::here(window, |w| std::mem::take(&mut w.pending_drops));
    for image in images {
        cx.drop_image(image, Some(window));
    }
}

/// One tick's events, in lottie-web's firing order, gated on the listener bits.
///
/// **The `loopComplete` that coincides with `complete` is dropped** — decision 2
/// in the module docs. `Advance::events()` reports the boundary that ended
/// playback as both, because at the crate's level it *is* both; lottie-web
/// fires only `complete` there, for `loop: false` and for a numeric `loop`
/// alike.
fn dispatch_events(id: u32, advance: &Advance, window: &mut Window, cx: &mut App) {
    let listeners = tree::read_tree(|t| t.node(id).map(|n| n.listeners)).unwrap_or(0);
    let listens = |ev: u32| listeners & (1u64 << ev) != 0;
    if listeners == 0 {
        return;
    }

    // `loops_completed` counts boundaries crossed this tick; the last of them
    // is the completing one when `completed` is set.
    let suppressed_loop = advance.completed.then_some(advance.loop_count);
    // lottie-web's `playCount` at `complete` does not count the completing
    // pass, so neither does the payload.
    let loops_done = if advance.completed {
        advance.loop_count.saturating_sub(1)
    } else {
        advance.loop_count
    };
    let total_frames = with_state(id, |s| s.meta().total_frames).unwrap_or(0.0) as f64;

    for event in advance.events() {
        match event {
            LottieEvent::EnterFrame { frame, elapsed } => {
                // High frequency: do not even build the payload unless someone
                // is listening (CONTRACT.md §2).
                if !listens(tree::EV_ENTERFRAME) {
                    continue;
                }
                let progress = with_state(id, |s| s.progress()).unwrap_or(0.0);
                let payload = EventPayload {
                    a: frame as f64,
                    b: total_frames,
                    c: elapsed as f64,
                    d: progress as f64,
                    key: None,
                };
                crate::render::js_dispatch_app(id, tree::EV_ENTERFRAME, &payload, window, cx);
            }
            LottieEvent::LoopComplete { count } => {
                if suppressed_loop == Some(count) || !listens(tree::EV_LOOPCOMPLETE) {
                    continue;
                }
                let payload = EventPayload {
                    a: with_state(id, |s| s.frame()).unwrap_or(0.0) as f64,
                    b: total_frames,
                    c: count as f64,
                    ..Default::default()
                };
                crate::render::js_dispatch_app(id, tree::EV_LOOPCOMPLETE, &payload, window, cx);
            }
            LottieEvent::Complete => {
                if !listens(tree::EV_COMPLETE) {
                    continue;
                }
                let payload = EventPayload {
                    a: with_state(id, |s| s.frame()).unwrap_or(0.0) as f64,
                    b: total_frames,
                    c: loops_done as f64,
                    ..Default::default()
                };
                crate::render::js_dispatch_app(id, tree::EV_COMPLETE, &payload, window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_follows_the_lottie_player_defaults_and_lottie_webs_counting() {
        // OWNER DECISION 1: absent means NOT looping (`<lottie-player>`), not
        // lottie-web's `loadAnimation` default.
        assert_eq!(loop_mode_from(None), LoopMode::None);
        assert_eq!(loop_mode_from(Some("false")), LoopMode::None);
        // Browser boolean-attribute spellings.
        assert_eq!(loop_mode_from(Some("")), LoopMode::Loop);
        assert_eq!(loop_mode_from(Some("true")), LoopMode::Loop);
        // OWNER DECISION 3: lottie-web's numeric `loop` counts repeats, so
        // `loop="2"` plays three times — Count(3) in the crate's counting.
        assert_eq!(loop_mode_from(Some("2")), LoopMode::Count(3));
        assert_eq!(loop_mode_from(Some("0")), LoopMode::None);
    }

    #[test]
    fn boolean_attributes_follow_the_html_rule() {
        assert!(!bool_attr(None));
        assert!(bool_attr(Some("")));
        assert!(bool_attr(Some("true")));
        assert!(!bool_attr(Some("false")));
    }

    #[test]
    fn segments_parse_from_the_string_spelling() {
        assert_eq!(segment_from(Some("10,40")), Some((10.0, 40.0)));
        assert_eq!(segment_from(Some(" 10 , 40 ")), Some((10.0, 40.0)));
        assert_eq!(segment_from(None), None);
        assert_eq!(segment_from(Some("")), None);
        assert_eq!(segment_from(Some("nonsense")), None);
    }
}
