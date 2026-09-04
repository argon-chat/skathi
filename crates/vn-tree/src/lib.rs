//! vn-tree — the retained node tree that Vue's custom renderer mutates through
//! `__vn_*` host functions, and the per-frame lowering of that tree into gpui
//! elements.
//!
//! Protocol (v0 — see docs/ARCHITECTURE.md §6):
//! - JS holds dense `u32` node ids (0 = none); names (tags, style keys, event
//!   names) are interned once via `__vn_intern` so the per-op hot path never
//!   carries a string.
//! - Structural ops apply synchronously (host calls cost ~3 ns); `insert` is
//!   also *move* (implicit detach), `remove` destroys the whole subtree.
//! - Mutations mark the tree dirty; the frame pump notifies the root view
//!   after the JS wave so gpui rebuilds from the tree in the same frame.
//!
//! Style handling here is a deliberate v0 placeholder: a hardcoded map of a
//! few inline properties onto `gpui::Styled` calls. The real CSS layer
//! (vn-css: stylesheets, cascade, per-state refinements) replaces it.

mod aria;
mod dom;
mod img;
pub mod log;
mod lottie;
pub mod perf;
mod render;
mod svg;
mod tree;
mod video;
pub mod windows;

pub mod devtools;

pub use img::{asset_base, set_asset_base};
pub use render::{
    bounds_in, build_root, build_subtree, control_hitboxes, control_hitboxes_in,
    enable_pick_hitboxes, flush_pending_mouse_move, painted_bounds_in, painted_drag_region,
    painted_drag_region_in, popped_out, root_bounds, root_bounds_in, set_os_chrome,
    set_os_chrome_in, set_popped_out, set_repaint_all,
};
pub use windows::WinKey;
pub use tree::{
    with_tree, EV_BLUR, EV_CANPLAY, EV_CANPLAYTHROUGH, EV_CLICK, EV_COMPLETE, EV_DURATIONCHANGE,
    EV_EMPTIED, EV_ENDED, EV_ENTERFRAME, EV_ERROR, EV_FOCUS, EV_INPUT, EV_KEYDOWN, EV_KEYUP,
    EV_LOADEDDATA, EV_LOADEDMETADATA, EV_LOADSTART, EV_LOOPCOMPLETE, EV_MOUSEDOWN, EV_MOUSEENTER,
    EV_MOUSELEAVE, EV_MOUSEMOVE, EV_MOUSEUP, EV_PAUSE, EV_PLAY, EV_PLAYING, EV_RATECHANGE,
    EV_SCROLL, EV_SEEKED, EV_SEEKING, EV_TIMEUPDATE, EV_VOLUMECHANGE, EV_WAITING, EV_WHEEL,
    EVENT_NAMES,
};

/// The numeric payload a native input event carries into JS. Meaning by event:
/// mouse: a=x b=y c=button d=modifier bits; wheel: a=deltaX b=deltaY d=mods;
/// key: d=mods + `key` (DOM name); input: `key` = the new text value;
/// scroll: a=scrollLeft b=scrollTop (CSS convention — positive px).
/// Modifier bits: 1 ctrl, 2 shift, 4 alt, 8 meta.
#[derive(Default)]
pub struct EventPayload<'a> {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub key: Option<&'a str>,
}

use std::ffi::c_void;

use vn_runtime::redjs::api::Value;
use vn_runtime::redjs::webapi::util::read_string;
use vn_runtime::{host_fn, JsRuntime};

const UNDEF: u64 = 0xa;

fn ret_id(id: u32) -> u64 {
    Value::int32(id as i32).0
}

fn arg_id(v: Value) -> u32 {
    v.as_number().unwrap_or(0.0) as u32
}

host_fn!(fn vn_intern(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    ret_id(tree::with_tree(|t| t.intern(&name)))
});

host_fn!(fn vn_create_element(_g, args) {
    let tag = arg_id(args.arg(0));
    ret_id(tree::with_tree(|t| t.create_element(tag)))
});

host_fn!(fn vn_create_text(_g, _args) {
    ret_id(tree::with_tree(|t| t.create_text()))
});

host_fn!(fn vn_create_anchor(_g, _args) {
    ret_id(tree::with_tree(|t| t.create_anchor()))
});

host_fn!(fn vn_set_text(g, args) {
    let id = arg_id(args.arg(0));
    let text = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    tree::with_tree(|t| t.set_text(id, text));
    UNDEF
});

host_fn!(fn vn_set_element_text(g, args) {
    let id = arg_id(args.arg(0));
    let text = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    tree::with_tree(|t| t.set_element_text(id, text));
    UNDEF
});

host_fn!(fn vn_insert(_g, args) {
    let (child, parent, anchor) = (arg_id(args.arg(0)), arg_id(args.arg(1)), arg_id(args.arg(2)));
    tree::with_tree(|t| t.insert(child, parent, anchor));
    UNDEF
});

host_fn!(fn vn_remove(_g, args) {
    let id = arg_id(args.arg(0));
    tree::with_tree(|t| t.remove(id));
    UNDEF
});

host_fn!(fn vn_set_style(g, args) {
    let id = arg_id(args.arg(0));
    let key = arg_id(args.arg(1));
    let value = unsafe { read_string(g, args.arg(2).0) }.unwrap_or_default();
    tree::with_tree(|t| t.set_style(id, key, value));
    UNDEF
});

host_fn!(fn vn_set_class(g, args) {
    let id = arg_id(args.arg(0));
    let classes = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    tree::with_tree(|t| t.set_class(id, &classes));
    UNDEF
});

host_fn!(fn vn_set_prop(g, args) {
    let id = arg_id(args.arg(0));
    let key = arg_id(args.arg(1));
    let value = if args.arg(2).is_undefined() || args.arg(2).is_null() {
        None
    } else {
        unsafe { read_string(g, args.arg(2).0) }
    };
    tree::with_tree(|t| t.set_prop(id, key, value));
    UNDEF
});

host_fn!(fn vn_add_scope(g, args) {
    let id = arg_id(args.arg(0));
    let scope = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    tree::with_tree(|t| t.add_scope(id, scope));
    UNDEF
});

host_fn!(fn vn_set_stylesheet(g, args) {
    let css = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    tree::set_stylesheet(css);
    // Every node needs a fresh cascade under the new sheet.
    tree::with_tree(|t| t.mark_dirty());
    UNDEF
});

host_fn!(fn vn_set_listener(g, args) {
    let id = arg_id(args.arg(0));
    let event = arg_id(args.arg(1));
    let on = args.arg(2).as_bool().unwrap_or(false);
    let _ = g;
    tree::with_tree(|t| t.set_listener(id, event, on));
    UNDEF
});

host_fn!(fn vn_create_root(_g, _args) {
    ret_id(tree::with_tree(|t| t.create_root()))
});

host_fn!(fn vn_set_dispatcher(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    tree::set_dispatcher(f.0);
    UNDEF
});

// ---------------------------------------------------------------------------
// the DOM surface (compat roadmap G2) — see `dom.rs` for the semantics
// ---------------------------------------------------------------------------

/// Builds a JS string Value for a host fn to return. The runtime thread-local
/// holds the same context the host call is running in, so this is the same
/// `Context` — not a second one.
fn ret_string(s: &str) -> u64 {
    JsRuntime::with(|rt| rt.ctx().string(s).0)
}

// `querySelectorAll` / `matches` / `closest`, answered by the REAL cascade
// matcher. Returns the matched ids comma-joined, `""` for no match, or
// `"!<advisory>"` when the selector is one the engine cannot run — the JS side
// turns that into the same advisory text a rule using it would print.
host_fn!(fn vn_query(g, args) {
    let selector = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let root = arg_id(args.arg(1));
    let mode = match arg_id(args.arg(2)) {
        1 => dom::QueryMode::SelfOnly,
        2 => dom::QueryMode::Closest,
        _ => dom::QueryMode::Descendants,
    };
    let out = match dom::query(&selector, root, mode) {
        Ok(ids) => ids.iter().map(u32::to_string).collect::<Vec<_>>().join(","),
        Err(e) => format!("!{e}"),
    };
    ret_string(&out)
});

// `getBoundingClientRect()` — `"x,y,w,h"`, or `""` when the node was not
// painted in the last frame (detached / `display: none`).
host_fn!(fn vn_bounds(_g, args) {
    let id = arg_id(args.arg(0));
    let out = match dom::bounds(id) {
        Some((x, y, w, h)) => format!("{x},{y},{w},{h}"),
        None => String::new(),
    };
    ret_string(&out)
});

// `getComputedStyle(el)` — `name=value` lines; see `dom::computed_style`.
host_fn!(fn vn_computed_style(_g, args) {
    let id = arg_id(args.arg(0));
    ret_string(&dom::computed_style(id).unwrap_or_default())
});

// ---------------------------------------------------------------------------
// the `<video>` media surface (crates/vn-video/CONTRACT.md §6)
// ---------------------------------------------------------------------------
//
// `HTMLMediaElement` is the one element whose interesting state lives in the
// HOST rather than in the mirror: `currentTime` moves sixty times a second
// inside the OS pipeline and no prop write puts it there. So the surface is a
// property read and a method call, the same split `__vn_bounds` /
// `__vn_computed_style` already established, and the JS runtime's job is only
// to spell them `el.currentTime` and `el.play()`.

// `name=value` lines, empty for a node that is not a loaded `<video>`.
host_fn!(fn vn_media_state(_g, args) {
    let id = arg_id(args.arg(0));
    ret_string(&video::media_state(id).unwrap_or_default())
});

// `play` / `pause` / `currentTime =` / `volume =` / `muted =` / `playbackRate =`
// / `loop =`, by verb. Returns whether the node was a live `<video>` at all.
host_fn!(fn vn_media_call(_g, args) {
    let id = arg_id(args.arg(0));
    let verb = arg_id(args.arg(1));
    let arg = args.arg(2).as_number().unwrap_or(0.0);
    Value::boolean(video::media_call(id, verb, arg)).0
});

// `canPlayType(type)` -> "" | "maybe" | "probably", probed live on this
// machine — never a static table (the owner constraint in vn_video's docs).
host_fn!(fn vn_media_can_play(g, args) {
    let id = arg_id(args.arg(0));
    let mime = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    ret_string(&video::can_play_type(id, &mime))
});

// Drains the node's recent media events, comma-joined. A bridge and a test
// surface — the events themselves go out on the ordinary EV_* channel.
host_fn!(fn vn_media_events(_g, args) {
    let id = arg_id(args.arg(0));
    ret_string(&video::drain_recent(id))
});

/// Registers the whole `__vn_*` protocol surface on the JS global.
pub fn install(rt: &JsRuntime) {
    let fns: &[(&str, u32, *const c_void)] = &[
        ("__vn_intern", 1, vn_intern as *const _),
        ("__vn_create_element", 1, vn_create_element as *const _),
        ("__vn_create_text", 0, vn_create_text as *const _),
        ("__vn_create_anchor", 0, vn_create_anchor as *const _),
        ("__vn_set_text", 2, vn_set_text as *const _),
        ("__vn_set_element_text", 2, vn_set_element_text as *const _),
        ("__vn_insert", 3, vn_insert as *const _),
        ("__vn_remove", 1, vn_remove as *const _),
        ("__vn_set_style", 3, vn_set_style as *const _),
        ("__vn_set_class", 2, vn_set_class as *const _),
        ("__vn_set_prop", 3, vn_set_prop as *const _),
        ("__vn_add_scope", 2, vn_add_scope as *const _),
        ("__vn_set_stylesheet", 1, vn_set_stylesheet as *const _),
        ("__vn_set_listener", 3, vn_set_listener as *const _),
        ("__vn_create_root", 0, vn_create_root as *const _),
        ("__vn_set_dispatcher", 1, vn_set_dispatcher as *const _),
        ("__vn_query", 3, vn_query as *const _),
        ("__vn_bounds", 1, vn_bounds as *const _),
        ("__vn_computed_style", 1, vn_computed_style as *const _),
        ("__vn_media_state", 1, vn_media_state as *const _),
        ("__vn_media_call", 3, vn_media_call as *const _),
        ("__vn_media_can_play", 2, vn_media_can_play as *const _),
        ("__vn_media_events", 1, vn_media_events as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

// ---------------------------------------------------------------------------
// the forced synchronous layout
// ---------------------------------------------------------------------------
//
// A browser lays out ON READ. `el.scrollHeight`, `getBoundingClientRect()` and
// the whole `offset*` / `client*` family flush whatever style and layout the
// document owes before they answer, which is why the single most common
// measurement idiom in every Vue app there is —
//
//     onMounted(() => nextTick(() => { box.style.height = inner.scrollHeight + "px" }))
//
// — works: the tree was built microseconds ago and has never been on screen,
// and the read lays it out anyway.
//
// Here, layout is what gpui does during a frame's prepaint, and the rectangles
// live in `render`'s per-window maps. JS that runs BEFORE the first frame —
// module evaluation, `app.mount()`, `onMounted`, and the microtasks Vue's
// `nextTick` drains at the end of that same script — was therefore reading maps
// that no draw had filled in yet, and got `0` from every one of those
// accessors. Not "stale": zero. `apps/meet-probe`'s join card measured its own
// panel that way, stored the 0 as the wrapper's `height`, and clipped its whole
// body away with the `overflow: hidden` beside it — laid out, hit-testable, and
// painting nothing. Nothing ever re-measured, because the app had its answer.
//
// So the read forces the layout, exactly as a browser's does. The host installs
// the hook (only it knows the window), and [`sync_layout`] is called from the
// one place that needs it — see [`dom::bounds`] for when, which is narrow on
// purpose: a node that has never been painted, while the tree still holds
// changes no frame has taken.

thread_local! {
    static SYNC_LAYOUT: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
    static IN_SYNC_LAYOUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Installs the host's "draw one frame, now" callback.
///
/// Only the host knows which gpui window the app root is painted by, and
/// `vn-tree` must not depend on `vn-app`, so the direction is inverted: the
/// host hands down a function and this crate decides when a DOM read has to
/// call it. Called once at startup.
pub fn set_sync_layout(f: fn()) {
    SYNC_LAYOUT.with(|c| c.set(Some(f)));
}

/// Folds the pending changes and draws one frame, so a layout read that follows
/// answers from rectangles rather than from an empty map.
///
/// Re-entrant calls are dropped: the draw walks the tree, and a layout read
/// from inside the walk would otherwise recurse forever. It is also a no-op
/// with no hook installed (`cargo test`, the devtools inspector process before
/// its window exists), so every caller still has to behave when the answer is
/// "not painted".
pub fn sync_layout() {
    if IN_SYNC_LAYOUT.with(|c| c.get()) {
        return;
    }
    let Some(f) = SYNC_LAYOUT.with(|c| c.get()) else { return };
    IN_SYNC_LAYOUT.with(|c| c.set(true));
    // The frame is about to paint from `node.computed`, so the cascade owes it
    // the same pass a JS wave's end would have run.
    flush_changes();
    f();
    IN_SYNC_LAYOUT.with(|c| c.set(false));
}

/// Call after a JS wave: if the tree changed, run the restyle pass (inline
/// parse + cascade) and report `true` so the caller notifies the root view.
pub fn flush_changes() -> bool {
    let dirty = tree::with_tree(|t| t.take_dirty());
    if dirty {
        tree::restyle_all();
        render::gc_node_state();
        tree::animator_gc();
    }
    dirty
}

/// True while any CSS transition runs — the host keeps repainting.
pub fn animating() -> bool {
    tree::animating()
}

/// True while any `<lottie>` still needs frames (crates/vn-lottie/CONTRACT.md
/// §4). Folded into the host's `animating()` OR-chain next to [`animating`], so
/// a playing animation keeps the pump awake and a paused one lets it sleep.
pub fn lotties_animating() -> bool {
    lottie::any_playing()
}

/// True while any `<video>` still needs frames (playing, seeking, or waiting
/// for its first frame). Folded into the host's `animating()` OR-chain next to
/// [`lotties_animating`], so a playing video keeps the pump awake and a paused
/// one lets it sleep.
pub fn videos_playing() -> bool {
    video::any_playing()
}

/// Publishes the renderer's D3D11 device to the `<video>` backend, so Media
/// Foundation decodes onto the texture pool gpui already owns instead of a
/// second one — which is what makes the zero-copy present path legal at all.
/// Without it `<video>` still works, on a private device, through the CPU
/// readback fallback (see `vn_video::mf`).
///
/// The host calls this **every frame**, cheaply, and reacts to the generation:
/// see [`video_device_changed`].
///
/// # Safety
/// `ptr` must be a live `ID3D11Device` COM pointer owned by the caller.
#[cfg(windows)]
pub unsafe fn set_video_device(ptr: *mut c_void, generation: u64) {
    vn_video::mf::set_shared_device(ptr, generation);
}

/// The renderer recreated its D3D11 device; rebuild every `<video>` on the new
/// one, keeping position and transport state. Returns how many were rebuilt.
///
/// Call it right after [`set_video_device`] on the frame the generation moved.
pub fn video_device_changed() -> usize {
    video::device_changed()
}

/// The per-frame work that needs a `Window` and cannot be done from a host fn.
/// Call it once per frame from the pump, BEFORE the draw; returns `true` when
/// something moved and the window must repaint.
///
/// Four jobs today:
/// * every `<lottie>` is advanced by the wall-clock delta and its events are
///   dispatched (CONTRACT.md §4) — this is also where retired atlas tiles go
///   back to gpui, since `flush_changes` has no `Window` to hand them to;
/// * every `<video>` is pulled for a frame and its media events are dispatched
///   (`crates/vn-video/CONTRACT.md` §3);
/// * the CSS animation clock is advanced and `transitionend` /
///   `animationstart` / `animationend` / `animationcancel` are dispatched;
/// * a node that was focused when it got detached from the painted tree loses
///   gpui focus, which is the browser's rule and what makes a `<KeepAlive>`
///   page stop swallowing keystrokes after it deactivates.
pub fn frame_tick(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    render::apply_pending_blur(window, cx);
    render::ensure_root_focus(window, cx);
    // `transitionend` / `animationstart` / `animationend` / `animationcancel`:
    // the animator queues them from the restyle pass and the paint clock, and
    // this is the one per-frame hook that has a `Window` to dispatch with.
    tree::advance_animations();
    // All of them, unconditionally: `|` rather than `||`, because a `<video>`
    // frame must be pulled even on a tick where a `<lottie>` also moved, and an
    // animation event must be dispatched even on a tick where both did.
    let anim = render::dispatch_anim_events(window, cx);
    let lottie_moved = lottie::tick(window, cx);
    let video_moved = video::tick(window, cx);
    anim | lottie_moved | video_moved
}

/// The per-frame work a SECONDARY window has to do for itself, because the
/// resource it touches belongs to that window and to no other
/// (docs/MULTIWINDOW.md §5.1).
///
/// Called from a pop-out's own `render()`, which is the only place that holds
/// its `&mut Window`. Three jobs, and each is a gpui resource the pump's window
/// physically cannot reach:
///
/// * **atlas tiles retired by nodes this window painted** go back to THIS
///   window's sprite atlas. `App::drop_image(image, Some(window))` sweeps every
///   other window as a courtesy, but the one it cannot reach through
///   `App::windows` is whichever window's update is currently running — so each
///   window drains its own queue;
/// * **zero-copy `<video>` frames** decoded by the pump are blitted into THIS
///   window's atlas, or the element paints transparent here;
/// * **focus**: a node detached while focused in THIS window loses gpui focus,
///   and this window keeps something on its own focus path so keystrokes have
///   somewhere to travel.
///
/// The JS-facing half of `frame_tick` — the lottie clock, the `<video>`
/// transport, the CSS animation clock — is deliberately NOT here: it is a
/// property of the one JS realm and must run once per FRAME GROUP, not once per
/// window (D-M06). Running it twice would double every `setTimeout` rate.
pub fn window_frame_tick(window: &mut gpui::Window, cx: &mut gpui::App) {
    lottie::drain_pending_drops_for(window, cx);
    video::drain_pending_drops_for(window, cx);
    video::drain_pending_uploads(window);
    render::apply_pending_blur(window, cx);
    render::ensure_root_focus(window, cx);
}

/// Forgets everything a window held, because it has closed.
///
/// Its sprite atlas went with it, so the tiles it still owed are simply
/// dropped: handing them to another window's `drop_image` would remove keys a
/// LIVE window may legitimately have re-inserted, which is a corruption rather
/// than a leak.
pub fn forget_window(key: windows::WinKey) {
    windows::forget(key);
}

/// Mirrors the OS reduced-motion preference into the transition animator.
pub fn set_reduce_motion(on: bool) {
    tree::set_reduce_motion(on);
}

/// Publishes the window environment `@media` conditions are evaluated against:
/// viewport size in CSS px, the OS/window colour scheme, the reduced-motion
/// preference, the DPI scale (`resolution`) and the chrome mode
/// (`display-mode`). Call it once per frame — it is a cheap compare, and on the
/// frame where something actually changed it marks the whole tree dirty so the
/// next [`flush_changes`] re-cascades every node (a resize or theme flip can
/// flip any `@media` rule in the sheet).
///
/// Returns `true` on the frames where the environment changed.
pub fn set_env(env: vn_css::Env) -> bool {
    let changed = tree::set_env(env);
    if changed {
        tree::with_tree(|t| t.mark_dirty());
    }
    changed
}

/// Publishes one SECONDARY window's `@media` environment (docs/MULTIWINDOW.md
/// §6). Same contract as [`set_env`], scoped to the subtree that window paints:
/// `true` on the frames where it changed, and the subtree is marked for a
/// re-cascade.
///
/// What this buys, exactly: `@media (max-width: …)`, `prefers-color-scheme`,
/// `resolution`, `display-mode` and `env(titlebar-area-*)` answer against the
/// window that PAINTS the element. What it does not: `vw`/`vh`/`rem` lengths,
/// which vue-native flattens to px at sheet-compile time against one
/// process-wide basis — see §6.2 and advisory A-M05.
pub fn set_env_for(key: windows::WinKey, env: vn_css::Env) -> bool {
    tree::set_window_env(key, env)
}

/// The environment the last [`set_env`] published — the source `matchMedia`
/// evaluates against, so JS and CSS can never disagree.
///
/// The PRIMARY window's, because there is one JS realm and therefore one
/// `matchMedia` (A-M04). [`set_env_for`] is the per-window half, and it feeds
/// the cascade rather than JS.
pub fn env() -> vn_css::Env {
    tree::env()
}

/// Evaluates one media-query string against the live [`env`], exactly as the
/// cascade would. `Err` carries the reason the prelude could not be compiled.
///
/// This is what makes `window.matchMedia` truthful: before it, `@media
/// (max-width: 600px)` worked in CSS while `matchMedia("(max-width: 600px)")`
/// answered `false` forever — the precise failure the compat-first policy
/// exists to prevent (docs/WINDOWING.md §1.6, D-09).
pub fn match_media(query: &str) -> Result<bool, String> {
    vn_css::match_media(query, &tree::env())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four inputs these tests actually vary; the DPI scale and the chrome
    /// mode come from the default.
    fn env(w: f32, h: f32, dark: bool, reduce_motion: bool) -> vn_css::Env {
        vn_css::Env {
            viewport_width: w,
            viewport_height: h,
            dark,
            reduce_motion,
            ..vn_css::Env::default()
        }
    }

    /// Each `#[test]` runs on its own thread, so the tree/engine/env
    /// thread-locals start fresh — no cross-test bleed.
    fn opacity_of(id: u32) -> Option<f32> {
        tree::read_tree(|t| {
            t.node(id)
                .and_then(|n| n.computed.as_ref())
                .and_then(|c| c.base.opacity)
        })
    }

    /// The contract the host relies on: a changed environment invalidates
    /// EVERYTHING (any `@media` rule in the sheet may have flipped), and an
    /// unchanged one costs nothing.
    #[test]
    fn env_changes_mark_the_whole_tree_dirty_exactly_once() {
        tree::with_tree(|t| t.take_dirty());
        assert!(set_env(env(800.0, 600.0, false, false)));
        assert!(tree::with_tree(|t| t.take_dirty()));
        // Same four values again: no change, no invalidation.
        assert!(!set_env(env(800.0, 600.0, false, false)));
        assert!(!tree::with_tree(|t| t.take_dirty()));
        // A theme flip alone counts…
        assert!(set_env(env(800.0, 600.0, true, false)));
        assert!(tree::with_tree(|t| t.take_dirty()));
        // …and so does a one-pixel resize.
        assert!(set_env(env(801.0, 600.0, true, false)));
        assert!(tree::with_tree(|t| t.take_dirty()));
    }

    /// End to end through the real tree: an element property drives
    /// `:disabled`, and a property write alone is enough to re-cascade.
    #[test]
    fn element_properties_drive_the_cascade() {
        let child = tree::with_tree(|t| {
            let root = t.create_root();
            let tag = t.intern("input");
            let child = t.create_element(tag);
            t.insert(child, root, 0);
            t.set_class(child, "field");
            child
        });
        tree::set_engine(vn_css::Engine::compile(
            ".field { opacity: 1 } .field:disabled { opacity: 0.4 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(child), Some(1.0));

        // Exactly what `patchProp` does for `:disabled="true"`.
        tree::with_tree(|t| {
            let key = t.intern("disabled");
            t.set_prop(child, key, Some("true".into()));
        });
        assert!(flush_changes(), "a prop write must dirty the tree");
        assert_eq!(opacity_of(child), Some(0.4));

        // …and `:disabled="false"` is not disabled, as in a browser.
        tree::with_tree(|t| {
            let key = t.intern("disabled");
            t.set_prop(child, key, Some("false".into()));
        });
        flush_changes();
        assert_eq!(opacity_of(child), Some(1.0));
    }

    /// `getComputedStyle` is a FLUSH POINT: a component may change an attribute
    /// and read the resulting style back on the next line, and CSSOM says it
    /// gets the new value. reka-ui's `<Presence>` does exactly that to decide
    /// whether an element has an exit animation to wait for, so a stale read
    /// unmounted every dialog, popover and drawer without animating it.
    #[test]
    fn getcomputedstyle_flushes_the_pending_cascade() {
        let child = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let child = t.create_element(div);
            t.insert(child, root, 0);
            let key = t.intern("data-state");
            t.set_prop(child, key, Some("open".into()));
            t.set_class(child, "box");
            child
        });
        tree::set_engine(vn_css::Engine::compile(
            ".box { animation: none } \
             .box[data-state=open] { animation: enter 150ms linear } \
             .box[data-state=closed] { animation: exit 150ms linear } \
             @keyframes enter { from { opacity: 0 } } @keyframes exit { to { opacity: 0 } }",
        ));
        tree::restyle_all();
        let name_now = || {
            dom::computed_style(child)
                .unwrap_or_default()
                .lines()
                .find_map(|l| l.strip_prefix("animation-name=").map(str::to_string))
                .unwrap_or_default()
        };
        assert_eq!(name_now(), "enter");

        // The flip, WITHOUT a frame in between — this is the `await nextTick()`
        // of the Presence watcher, not a repaint.
        tree::with_tree(|t| {
            let key = t.intern("data-state");
            t.set_prop(child, key, Some("closed".into()));
        });
        assert_eq!(name_now(), "exit", "the read must see its own mutation");

        // …and the wave still reports a change, so the frame still repaints:
        // the flush must not swallow the dirty flag it consumed.
        assert!(flush_changes(), "a forced flush still owes the frame a repaint");
    }

    /// Custom properties inherit through the REAL tree, which is what obliges
    /// the restyle pass to walk in document order rather than by node id.
    #[test]
    fn custom_properties_inherit_through_the_real_tree() {
        let (mid, leaf) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mid = t.create_element(div);
            let leaf = t.create_element(div);
            // Deliberately inserted CHILD-FIRST so the id order and the document
            // order disagree — the pass must follow the tree, not the slab.
            t.insert(leaf, mid, 0);
            t.insert(mid, root, 0);
            t.set_class(mid, "page");
            t.set_class(leaf, "leaf");
            (mid, leaf)
        });
        tree::set_engine(vn_css::Engine::compile(
            ":root { --v: 0.2 } .page { --v: 0.8 } .leaf { opacity: var(--v) }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(leaf), Some(0.8));

        // Removing the override falls back to the root's value — i.e. the child
        // really re-reads its ancestors rather than caching a literal.
        tree::with_tree(|t| t.set_class(mid, "other"));
        flush_changes();
        assert_eq!(opacity_of(leaf), Some(0.2));
    }

    /// `element.style.setProperty("--x", …)` — reka-ui's `--reka-popper-*`
    /// channel — reaches the map and the subtree that reads it.
    #[test]
    fn an_inline_custom_property_reaches_the_subtree() {
        let leaf = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let host = t.create_element(div);
            let leaf = t.create_element(div);
            t.insert(host, root, 0);
            t.insert(leaf, host, 0);
            t.set_class(leaf, "leaf");
            let key = t.intern("--v");
            t.set_style(host, key, "0.35".into());
            leaf
        });
        tree::set_engine(vn_css::Engine::compile(".leaf { opacity: var(--v, 1) }"));
        tree::restyle_all();
        assert_eq!(opacity_of(leaf), Some(0.35));
    }

    // -----------------------------------------------------------------------
    // restyle scoping (docs/ARCHITECTURE.md §11)
    //
    // A mutation recascades the mutated node's SUBTREE, not the document. These
    // are the four ways that could be wrong.
    // -----------------------------------------------------------------------

    /// A descendant combinator reads every ancestor, so a class change on a
    /// container has to reach the leaf under it — the subtree walk is exactly
    /// what makes that hold.
    #[test]
    fn a_class_change_restyles_the_whole_subtree_under_it() {
        let (mid, leaf) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mid = t.create_element(div);
            let leaf = t.create_element(div);
            t.insert(mid, root, 0);
            t.insert(leaf, mid, 0);
            t.set_class(leaf, "leaf");
            (mid, leaf)
        });
        tree::set_engine(vn_css::Engine::compile(
            ".leaf { opacity: 0.1 } .page .leaf { opacity: 0.9 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(leaf), Some(0.1));

        // Only `mid` is touched; the leaf two nodes down must still move.
        tree::with_tree(|t| t.set_class(mid, "page"));
        flush_changes();
        assert_eq!(opacity_of(leaf), Some(0.9));
    }

    /// The same, through the custom-property channel: an inherited `--x`
    /// redefined on an ancestor has to re-resolve every `var(--x)` beneath it.
    #[test]
    fn an_inherited_var_flip_restyles_the_descendants_that_read_it() {
        let (mid, leaf) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mid = t.create_element(div);
            let leaf = t.create_element(div);
            t.insert(mid, root, 0);
            t.insert(leaf, mid, 0);
            t.set_class(mid, "a");
            t.set_class(leaf, "leaf");
            (mid, leaf)
        });
        tree::set_engine(vn_css::Engine::compile(
            ".a { --v: 0.25 } .b { --v: 0.75 } .leaf { opacity: var(--v, 1) }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(leaf), Some(0.25));

        tree::with_tree(|t| t.set_class(mid, "b"));
        flush_changes();
        assert_eq!(opacity_of(leaf), Some(0.75));
    }

    /// `+` and `~` are the one combinator that reads SIDEWAYS, so a sheet that
    /// uses them makes a mark reach its following siblings too.
    #[test]
    fn a_sibling_combinator_still_sees_a_class_change_next_door() {
        let (first, second) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let first = t.create_element(div);
            let second = t.create_element(div);
            t.insert(first, root, 0);
            t.insert(second, root, 0);
            t.set_class(second, "second");
            (first, second)
        });
        tree::set_engine(vn_css::Engine::compile(
            ".second { opacity: 0.2 } .marker + .second { opacity: 0.8 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(second), Some(0.2));

        // Nothing about `second` changed — only the element BEFORE it.
        tree::with_tree(|t| t.set_class(first, "marker"));
        flush_changes();
        assert_eq!(opacity_of(second), Some(0.8));
    }

    /// A structural pseudo reads the parent's child list, so an insert has to
    /// recascade the siblings around it — which is why every insert/remove
    /// marks the PARENT rather than the moved node.
    #[test]
    fn inserting_a_child_recascades_the_siblings_a_structural_pseudo_reads() {
        let (parent, only) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let parent = t.create_element(div);
            let only = t.create_element(div);
            t.insert(parent, root, 0);
            t.insert(only, parent, 0);
            t.set_class(only, "item");
            (parent, only)
        });
        tree::set_engine(vn_css::Engine::compile(
            ".item { opacity: 0.3 } .item:last-child { opacity: 0.7 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(only), Some(0.7), "the only child is the last child");

        // Appending a second child makes the first one stop being `:last-child`
        // even though nothing on it was written.
        tree::with_tree(|t| {
            let div = t.intern("div");
            let second = t.create_element(div);
            t.insert(second, parent, 0);
            t.set_class(second, "item");
        });
        flush_changes();
        assert_eq!(opacity_of(only), Some(0.3));
    }

    /// Past the mark limit the pass falls back to one straight document walk.
    /// The cliff must be invisible: everything still ends up correct, including
    /// the nodes whose marks were dropped on the way over the edge.
    #[test]
    fn a_wave_that_marks_more_than_the_limit_still_restyles_everything() {
        let ids = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let ids: Vec<u32> = (0..200)
                .map(|_| {
                    let id = t.create_element(div);
                    t.insert(id, root, 0);
                    id
                })
                .collect();
            ids
        });
        tree::set_engine(vn_css::Engine::compile(".on { opacity: 0.6 }"));
        tree::restyle_all();

        // 200 separate class writes — well past SCOPED_RESTYLE_LIMIT.
        tree::with_tree(|t| {
            for &id in &ids {
                t.set_class(id, "on");
            }
        });
        flush_changes();
        for &id in &ids {
            assert_eq!(opacity_of(id), Some(0.6), "node {id} was left unstyled");
        }
    }

    /// `@media` conditions read the published env, and moving it re-cascades.
    #[test]
    fn media_conditions_follow_the_published_env() {
        let child = tree::with_tree(|t| {
            let root = t.create_root();
            let tag = t.intern("div");
            let child = t.create_element(tag);
            t.insert(child, root, 0);
            t.set_class(child, "box");
            child
        });
        tree::set_engine(vn_css::Engine::compile(
            ".box { opacity: 0.1 } @media (min-width: 640px) { .box { opacity: 0.9 } }",
        ));

        set_env(env(500.0, 600.0, false, false));
        flush_changes();
        assert_eq!(opacity_of(child), Some(0.1));

        // The resize alone — no tree mutation at all — must re-cascade.
        assert!(set_env(env(700.0, 600.0, false, false)));
        assert!(flush_changes());
        assert_eq!(opacity_of(child), Some(0.9));
    }

    fn inert_of(id: u32) -> Option<bool> {
        tree::read_tree(|t| {
            t.node(id).and_then(|n| n.computed.as_ref()).map(|c| {
                c.extras.pointer_events_none.expect("the cascade resolves this for every element")
            })
        })
    }

    /// `pointer-events` is an INHERITED property, and the whole "invisible
    /// panel parked over the visible one" idiom depends on it: an
    /// `opacity-0 pointer-events-none` tab panel is `absolute inset-x-0 top-0`,
    /// so its controls sit pixel for pixel on the live ones and — being painted
    /// later — take every click aimed at them. On apps/meet-probe that is why
    /// typing into "Your name" did nothing.
    ///
    /// A descendant can still opt back IN, which is exactly what reka's
    /// dismissable layer does: it sets `body { pointer-events: none }` and
    /// `pointer-events: auto` on the open dialog.
    #[test]
    fn pointer_events_none_inherits_and_can_be_opted_out_of() {
        let (panel, child, opted_in, deep) = tree::with_tree(|t| {
            let root = t.create_root();
            let tag = t.intern("div");
            let mut kid = |parent: u32, class: &str| {
                let id = t.create_element(tag);
                t.insert(id, parent, 0);
                t.set_class(id, class);
                id
            };
            let panel = kid(root, "inert");
            let child = kid(panel, "plain");
            let opted_in = kid(panel, "live");
            let deep = kid(opted_in, "plain");
            (panel, child, opted_in, deep)
        });
        tree::set_engine(vn_css::Engine::compile(
            ".inert { pointer-events: none } .live { pointer-events: auto }",
        ));
        flush_changes();

        assert_eq!(inert_of(panel), Some(true));
        assert_eq!(inert_of(child), Some(true), "inertness inherits");
        assert_eq!(inert_of(opted_in), Some(false), "`auto` opts back in");
        assert_eq!(inert_of(deep), Some(false), "…and that inherits too");
    }
}

/// Dispatches an input event into JS (through the registered dispatcher) and
/// drains microtasks so Vue's reactive flush lands in the same frame. Must be
/// called with `Phase::Commit` published. Returns `false` when a JS handler
/// called `preventDefault()`.
pub fn dispatch_event(node: u32, event: u32, payload: &EventPayload) -> bool {
    let f = tree::dispatcher();
    if f == 0 {
        return true;
    }
    let span = perf::Span::start();
    let proceed = dispatch_event_inner(f, node, event, payload);
    perf::note_js(span.ms());
    proceed
}

fn dispatch_event_inner(f: u64, node: u32, event: u32, payload: &EventPayload) -> bool {
    JsRuntime::with(|rt| {
        let key = payload.key.map(|k| rt.ctx().string(k));
        let args = [
            Value::int32(node as i32),
            Value::int32(event as i32),
            Value::double(payload.a),
            Value::double(payload.b),
            Value::double(payload.c),
            Value::double(payload.d),
            key.unwrap_or_else(Value::undefined),
        ];
        let proceed = match rt.ctx().call(Value(f), Value::undefined(), &args) {
            Ok(v) => v.as_bool().unwrap_or(true),
            Err(vn_runtime::redjs::api::Error::Exception(e)) => {
                let err = rt.describe(e);
                eprintln!("[js dispatch] {}\n{}", err.header, err.stack);
                log::record("js", format!("[js dispatch] {}\n{}", err.header, err.stack));
                true
            }
            Err(_) => true,
        };
        // Vue schedules its patch flush on Promise microtasks: drain now so
        // the DOM ops run inside this very listener's Commit publication.
        rt.ctx().drain_microtasks();
        proceed
    })
}
