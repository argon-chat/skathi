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
mod bgimage;
mod cache;
mod clipboard;
mod dom;
mod fonts;
mod img;
pub mod log;
mod lottie;
pub mod perf;
mod render;
mod resample;
mod scroll;
mod svg;
mod tree;
mod video;
pub mod windows;

pub mod devtools;

pub use img::{asset_base, asset_store, content_key, set_asset_base, set_asset_store, AssetLookup};
pub use render::{
    bounds_in, build_root, build_subtree, control_hitboxes, control_hitboxes_in,
    enable_pick_hitboxes, flush_pending_mouse_move, painted_bounds_in, painted_drag_region,
    painted_drag_region_in, popped_out, root_bounds, root_bounds_in, set_os_chrome,
    set_os_chrome_in, set_popped_out, set_repaint_all,
};
pub use windows::WinKey;
pub use tree::{
    with_tree, EV_BLUR, EV_CANPLAY, EV_CANPLAYTHROUGH, EV_CLICK, EV_COMPLETE, EV_CONTEXTMENU,
    EV_COPY, EV_CUT, EV_DURATIONCHANGE, EV_PASTE,
    EV_EMPTIED, EV_ENDED, EV_ENTERFRAME, EV_ERROR, EV_FOCUS, EV_INPUT, EV_KEYDOWN, EV_KEYUP,
    EV_LOAD, EV_LOADEDDATA, EV_LOADEDMETADATA, EV_LOADSTART, EV_LOOPCOMPLETE, EV_MOUSEDOWN,
    EV_MOUSEENTER, EV_MOUSELEAVE, EV_MOUSEMOVE, EV_MOUSEUP, EV_PAUSE, EV_PLAY, EV_PLAYING,
    EV_POINTERCANCEL, EV_RATECHANGE, EV_SCROLL, EV_SEEKED, EV_SEEKING, EV_TIMEUPDATE,
    EV_VOLUMECHANGE, EV_WAITING, EV_WHEEL, EVENT_NAMES,
};

/// The numeric payload a native input event carries into JS. Meaning by event:
/// mouse: a=x b=y c=button d=modifier bits, plus `detail` (the click count) in
/// the bits above them (`d = mods | detail << 4`, see `render::mouse_payload`);
/// wheel: a=deltaX b=deltaY d=mods; key: d=mods + `key` (DOM name); input:
/// `key` = the new text value; scroll: a=scrollLeft b=scrollTop (CSS
/// convention — positive px); `<img>` load: a/b = natural size, `key` = src.
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

// Does the engine take `name: value` as a declaration it applies
// (`vn_css::accepts_declaration`)? The array form of `:style`
// (`display: ["-webkit-box", "flex"]`) keeps the last value the browser
// accepts, and only the engine can say which one that is here.
host_fn!(fn vn_style_accepts(g, args) {
    let key = arg_id(args.arg(0));
    let value = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let name = tree::with_tree(|t| t.name(key).to_string());
    Value::boolean(vn_css::accepts_declaration(&name, &value)).0
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
    let img_src = tree::with_tree(|t| {
        t.set_prop(id, key, value);
        t.name(key) == "src" && t.node(id).is_some_and(|n| t.name(n.tag) == "img")
    });
    // An `<img>`'s `src` is answered with `load` / `error` (`img::tick`),
    // whether or not the element is ever painted.
    if img_src {
        img::note_src(id);
    }
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
// the layout signal (m26 client sweep #27)
// ---------------------------------------------------------------------------
//
// ResizeObserver and IntersectionObserver sample layout, and layout only
// changes in a frame the host builds. Polling every frame regardless kept an
// idle app with an observer running JS at the display's rate. So once a poll
// finds nothing changed, the runtime parks it (`__vn_layout_watch`), and the
// host calls the runtime's hook (`__vn_set_layout_hook`) once, from the first
// frame tick after the next build.

thread_local! {
    /// The runtime's layout hook, a protected JS function; 0 = none.
    static LAYOUT_HOOK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The build count the parked runtime has seen; `None` = not parked.
    static LAYOUT_WATCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

host_fn!(fn vn_set_layout_hook(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    let old = LAYOUT_HOOK.with(|c| c.replace(f.0));
    if old != 0 {
        unsafe { vn_runtime::redjs::ffi::redjs_unprotect(g, old) };
    }
    UNDEF
});

host_fn!(fn vn_layout_watch(_g, _args) {
    watch_layout();
    UNDEF
});

/// Parks the runtime's samplers until the next build in any window.
fn watch_layout() {
    LAYOUT_WATCH.with(|c| c.set(Some(render::layout_count())));
}

/// `true` once per park, on the first call after a build: the watch is taken.
fn take_layout_watch() -> bool {
    let Some(seen) = LAYOUT_WATCH.with(|c| c.get()) else { return false };
    if render::layout_count() == seen {
        return false;
    }
    LAYOUT_WATCH.with(|c| c.set(None));
    true
}

/// Calls the runtime's layout hook if it is parked and a frame has been built
/// since. Once per park: the runtime parks again when its polls go quiet.
fn wake_layout_watchers(window: &mut gpui::Window, cx: &mut gpui::App) {
    if !take_layout_watch() {
        return;
    }
    let f = LAYOUT_HOOK.with(|c| c.get());
    if f == 0 {
        return;
    }
    render::js_wave_app(window, cx, || {
        JsRuntime::with(|rt| {
            if let Err(vn_runtime::redjs::api::Error::Exception(e)) =
                rt.ctx().call(Value(f), Value::undefined(), &[])
            {
                let err = rt.describe(e);
                log::record("js", format!("[layout hook] {}\n{}", err.header, err.stack));
            }
            rt.ctx().drain_microtasks();
        });
        true
    });
}

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
// painted in the last frame (detached / `display: none`), or `"stale"` when it
// has no rectangle YET and the layout it is owed could not be forced (a gpui
// input listener is on the stack; see `dom::bounds`).
host_fn!(fn vn_bounds(_g, args) {
    let id = arg_id(args.arg(0));
    let out = match dom::bounds(id) {
        (Some((x, y, w, h)), _) => format!("{x},{y},{w},{h}"),
        (None, true) => "stale".to_string(),
        (None, false) => String::new(),
    };
    ret_string(&out)
});

// `getComputedStyle(el)` — `name=value` lines; see `dom::computed_style`.
host_fn!(fn vn_computed_style(_g, args) {
    let id = arg_id(args.arg(0));
    ret_string(&dom::computed_style(id).unwrap_or_default())
});

// ---------------------------------------------------------------------------
// programmatic scrolling (crates/vn-tree/src/scroll.rs)
// ---------------------------------------------------------------------------
//
// One call for the whole of `scrollTop` / `scrollLeft` / `scrollTo` /
// `scrollBy` / `scrollWidth` / `scrollHeight`, because all six are the same
// question asked of the same `ScrollHandle` and a round trip per accessor
// would cost more than the arithmetic.
//
//   op 0  read            — x, y ignored
//   op 1  set absolute    — NaN on an axis leaves it alone (`scrollTo({top})`)
//   op 2  scroll by       — NaN is 0
//
// Answers `"left,top,scrollWidth,scrollHeight"` in CSS px, or `""` for a node
// the cascade never gave `overflow: scroll|auto` — which is not an error: a
// browser reports 0 for `scrollTop` on every non-scrollable element and
// ignores a write to it.
host_fn!(fn vn_scroll(_g, args) {
    let id = arg_id(args.arg(0));
    let op = arg_id(args.arg(1));
    let x = args.arg(2).as_number().unwrap_or(f64::NAN);
    let y = args.arg(3).as_number().unwrap_or(f64::NAN);
    let axis = |v: f64| (!v.is_nan()).then_some(v as f32);
    let out = match op {
        1 => scroll::set(id, axis(x), axis(y)),
        2 => scroll::by(id, axis(x).unwrap_or(0.0), axis(y).unwrap_or(0.0)),
        _ => scroll::info(id),
    };
    ret_string(&match out {
        Some(i) => format!(
            "{},{},{},{},{}",
            i.left,
            i.top,
            i.scroll_width,
            i.scroll_height,
            u8::from(i.stale)
        ),
        None => String::new(),
    })
});

// `element.scrollIntoView({ block, inline })` — 0 start, 1 center, 2 end,
// 3 nearest. `""` when it scrolled (or had nothing to do), `!<advisory>` when
// it could not, in the same shape `__vn_query` uses for a selector the engine
// cannot run.
host_fn!(fn vn_scroll_into_view(_g, args) {
    let id = arg_id(args.arg(0));
    let block = scroll::Align::from_code(arg_id(args.arg(1)));
    let inline = scroll::Align::from_code(arg_id(args.arg(2)));
    ret_string(&match scroll::into_view(id, block, inline) {
        Ok(false) => String::new(),
        Ok(true) => "stale".to_string(),
        Err(e) => format!("!{e}"),
    })
});

// ---------------------------------------------------------------------------
// programmatic focus (render.rs, "Programmatic focus")
// ---------------------------------------------------------------------------
//
// `__vn_focus(id, op)`:
//
//   op 0  `el.focus()` -> "moved:<old id>" (focus moved now; JS fires blur/
//         focusout on <old> unless it is 0, then focus/focusin) | "already" |
//         "pending" (queued for the frame tick; the host's observers fire the
//         events) | "refused" (not focusable, or not in the document)
//   op 1  `el.blur()`  -> "blurred" (JS fires blur/focusout) | ""
//
// Outside a Commit publication (no window: module-scope code at startup), a
// focus is queued and a blur is a no-op.
host_fn!(fn vn_focus(_g, args) {
    let id = arg_id(args.arg(0));
    let op = arg_id(args.arg(1));
    // A node mounted in this same update has no handle yet: lay out first
    // where that is legal, so the focus is synchronous (render::focus_wants_build).
    // Under a gpui listener this draws nothing and the focus is queued (FO-1).
    if op == 0 && layout_owed() && render::focus_wants_build(id) {
        sync_layout();
    }
    let out = vn_runtime::with_commit(|window, cx| {
        if op == 1 {
            return if render::js_blur(id, window, cx) { "blurred".to_string() } else { String::new() };
        }
        match render::js_focus(id, window, cx) {
            render::FocusAnswer::Moved { old } => format!("moved:{old}"),
            render::FocusAnswer::Already => "already".to_string(),
            render::FocusAnswer::Pending => "pending".to_string(),
            render::FocusAnswer::Refused => "refused".to_string(),
        }
    })
    .unwrap_or_else(|| {
        if op == 1 {
            return String::new();
        }
        render::queue_focus(id);
        "pending".to_string()
    });
    ret_string(&out)
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

// `drawImage(img)` / `createImageBitmap(img)` (packages/vue-native/src/canvas.ts):
// the `<img>`'s pixels at its natural size as an ImageBitmap, `null` when there
// is nothing to draw yet, or a string saying why the image cannot be drawn.
// Argument 1 is HTML's origin-clean flag for the image, which the bitmap
// carries into the canvas that draws it.
host_fn!(fn vn_img_bitmap(g, args) {
    let id = arg_id(args.arg(0));
    let clean = args.arg(1).as_bool().unwrap_or(true);
    let found = vn_runtime::with_app(|cx| img::pixels(id, Some(cx))).unwrap_or_else(|| img::pixels(id, None));
    match found {
        img::Pixels::Ready { width, height, rgba } => {
            let mut data = vn_runtime::redjs::webapi::image::BitmapData::new(width, height, rgba);
            data.origin_clean = clean;
            unsafe { vn_runtime::redjs::webapi::image::create_bitmap(g, data) }
        }
        img::Pixels::NotYet => Value::null().0,
        img::Pixels::Broken(why) => ret_string(&why),
    }
});

// ---------------------------------------------------------------------------
// web fonts (m26 client sweep #29) — see `fonts.rs`; the JS half is
// packages/vue-native/src/fonts.ts (`FontFace`, `document.fonts`)
// ---------------------------------------------------------------------------

// The page's `(id, status, message)` callback for face status changes.
host_fn!(fn vn_fonts_set_listener(g, args) {
    let f = args.arg(0);
    unsafe { vn_runtime::redjs::ffi::redjs_protect(g, f.0) };
    let old = fonts::listener();
    fonts::set_listener(f.0);
    if old != 0 {
        unsafe { vn_runtime::redjs::ffi::redjs_unprotect(g, old) };
    }
    UNDEF
});

// Every face, as JSON (`fonts::list_json`).
host_fn!(fn vn_fonts_list(_g, _args) {
    ret_string(&fonts::list_json())
});

// `new FontFace(family, source, descriptors)`: (family, CSS src text, the
// descriptors packed as "style|weight|stretch|unicodeRange|display", load
// now?) -> the face's id, or a string saying why it is a SyntaxError.
host_fn!(fn vn_font_create(g, args) {
    let family = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let source = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let packed = unsafe { read_string(g, args.arg(2).0) }.unwrap_or_default();
    let now = args.arg(3).as_bool().unwrap_or(false);
    match fonts::descriptors(&family, &packed).and_then(|d| fonts::create(d, &source, now)) {
        Ok(id) => ret_id(id),
        Err(why) => ret_string(&why),
    }
});

// `FontFace.load()`.
host_fn!(fn vn_font_load(_g, args) {
    fonts::load(arg_id(args.arg(0)));
    UNDEF
});

// `document.fonts.add(face)` / `.delete(face)`.
host_fn!(fn vn_font_set(_g, args) {
    fonts::set_in_set(arg_id(args.arg(0)), args.arg(1).as_bool().unwrap_or(false));
    UNDEF
});

// `document.fonts.load(font, text)` / `.check(font, text)`: the ids of the
// faces in the set that the `font` shorthand and the text select, comma-joined
// ("" for none), or null when `font` is not a valid shorthand.
host_fn!(fn vn_fonts_match(g, args) {
    let font = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let text = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    match fonts::matching(&font, &text) {
        Some(ids) => ret_string(&ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")),
        None => Value::null().0,
    }
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
        ("__vn_style_accepts", 2, vn_style_accepts as *const _),
        ("__vn_set_class", 2, vn_set_class as *const _),
        ("__vn_set_prop", 3, vn_set_prop as *const _),
        ("__vn_add_scope", 2, vn_add_scope as *const _),
        ("__vn_set_stylesheet", 1, vn_set_stylesheet as *const _),
        ("__vn_set_listener", 3, vn_set_listener as *const _),
        ("__vn_create_root", 0, vn_create_root as *const _),
        ("__vn_set_dispatcher", 1, vn_set_dispatcher as *const _),
        ("__vn_set_layout_hook", 1, vn_set_layout_hook as *const _),
        ("__vn_layout_watch", 0, vn_layout_watch as *const _),
        ("__vn_query", 3, vn_query as *const _),
        ("__vn_bounds", 1, vn_bounds as *const _),
        ("__vn_computed_style", 1, vn_computed_style as *const _),
        ("__vn_scroll", 4, vn_scroll as *const _),
        ("__vn_scroll_into_view", 3, vn_scroll_into_view as *const _),
        ("__vn_focus", 2, vn_focus as *const _),
        ("__vn_media_state", 1, vn_media_state as *const _),
        ("__vn_media_call", 3, vn_media_call as *const _),
        ("__vn_media_can_play", 2, vn_media_can_play as *const _),
        ("__vn_media_events", 1, vn_media_events as *const _),
        ("__vn_img_bitmap", 2, vn_img_bitmap as *const _),
        ("__vn_fonts_set_listener", 1, vn_fonts_set_listener as *const _),
        ("__vn_fonts_list", 0, vn_fonts_list as *const _),
        ("__vn_font_create", 4, vn_font_create as *const _),
        ("__vn_font_load", 1, vn_font_load as *const _),
        ("__vn_font_set", 2, vn_font_set as *const _),
        ("__vn_fonts_match", 2, vn_fonts_match as *const _),
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
    /// How many JS dispatches a gpui listener (or gpui's own draw) has on the
    /// stack ([`InputDispatch`]). See [`in_input_dispatch`].
    static INPUT_DISPATCH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Inside a dispatch no gpui listener is under ([`off_input`]): the frame
    /// tick, the pump's coalesced `mousemove`, the devtools' synthesized
    /// events, a `window.defer` callback.
    static OFF_INPUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// [`render::build_count`] when [`flush_changes`] last took changes: while
    /// it still equals the build count, the restyled tree has not been laid out.
    static FLUSHED_AT_BUILD: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    /// A forced layout took the wave's changes ([`sync_layout`] flushes them
    /// for its draw), so the next [`flush_changes`] must still report them:
    /// the host repaints and presents on that answer, and a draw that is never
    /// presented would leave the window showing the frame before.
    static FLUSHED_BY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Is a gpui listener on the stack that called into JS (a mouse, key, wheel or
/// clipboard handler), or gpui's own draw (`focus` / `blur`)?
///
/// gpui takes the rendered frame's mouse listeners out of the frame for the
/// whole of a mouse dispatch and puts them back afterwards, and a key dispatch
/// walks the rendered frame's dispatch tree. A draw in the middle would swap
/// the frames under both: the old listeners would be written into the new
/// frame. Focus listeners run INSIDE `Window::draw`, after its frame swap, and
/// a draw there would nest. So the host's [`sync_layout`] hook does not draw
/// while this is true, and a layout read there stays stale
/// ([`dom::bounds`]).
pub fn in_input_dispatch() -> bool {
    INPUT_DISPATCH.with(|c| c.get()) > 0
}

/// Marks a JS dispatch that a gpui listener started; a no-op under
/// [`off_input`] (see [`in_input_dispatch`]).
pub(crate) struct InputDispatch(bool);

impl InputDispatch {
    pub(crate) fn enter() -> Self {
        if OFF_INPUT.with(|c| c.get()) {
            return Self(false);
        }
        Self::inside_gpui()
    }

    /// The same mark, whatever encloses it: for a listener gpui calls from
    /// inside its draw, which no enclosing mark can make safe to draw under.
    pub(crate) fn inside_gpui() -> Self {
        INPUT_DISPATCH.with(|c| c.set(c.get() + 1));
        Self(true)
    }
}

impl Drop for InputDispatch {
    fn drop(&mut self) {
        if self.0 {
            INPUT_DISPATCH.with(|c| c.set(c.get().saturating_sub(1)));
        }
    }
}

/// Runs `f`, whose JS dispatches no gpui listener is under: the frame tick,
/// the pump's coalesced `mousemove` and the devtools' synthesized events (all
/// in the pump, before gpui's draw), and a `window.defer` callback (gpui runs
/// it once the dispatch that queued it has returned and put the frame's
/// listeners back). A forced layout may draw there. The mark only keeps a
/// dispatch from COUNTING as input; an enclosing input dispatch still counts.
/// None of them synthesizes input through `Window::dispatch_event`; one that
/// did would have to leave this mark first.
pub(crate) fn off_input<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            OFF_INPUT.with(|c| c.set(self.0));
        }
    }
    let _r = Restore(OFF_INPUT.with(|c| c.replace(true)));
    f()
}

/// Does the tree hold changes no frame has laid out yet? Either they have not
/// been flushed, or they were flushed and no element-tree build followed (a
/// forced layout that the host could not draw).
pub(crate) fn layout_owed() -> bool {
    tree::read_tree(|t| t.is_dirty()) || FLUSHED_AT_BUILD.with(|c| c.get()) == Some(render::build_count())
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
    if flush_changes() {
        FLUSHED_BY_SYNC.with(|c| c.set(true));
    }
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
        render::canvas::gc(|id| tree::read_tree(|t| t.node(id).is_some()));
        tree::animator_gc();
        FLUSHED_AT_BUILD.with(|c| c.set(Some(render::build_count())));
    }
    // Inside a forced layout this is its own flush; outside, it also answers
    // for the changes a forced layout took earlier in the wave.
    let forced = !IN_SYNC_LAYOUT.with(|c| c.get()) && FLUSHED_BY_SYNC.with(|c| c.replace(false));
    dirty || forced
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
/// Eight jobs today:
/// * `blob:` image decodes that no node shows any more, or that belong to a
///   file picked again after it changed, are handed back to gpui's asset
///   cache (`img::evict_pending`);
/// * every `<lottie>` is advanced by the wall-clock delta and its events are
///   dispatched (CONTRACT.md §4) — this is also where retired atlas tiles go
///   back to gpui, since `flush_changes` has no `Window` to hand them to;
/// * every `<video>` is pulled for a frame and its media events are dispatched
///   (`crates/vn-video/CONTRACT.md` §3);
/// * a `<canvas>` that drew since the last frame asks for a repaint;
/// * the CSS animation clock is advanced and `transitionend` /
///   `animationstart` / `animationend` / `animationcancel` are dispatched;
/// * a node that was focused when it got detached from the painted tree loses
///   gpui focus, which is the browser's rule and what makes a `<KeepAlive>`
///   page stop swallowing keystrokes after it deactivates;
/// * web fonts that finished loading are handed to gpui's text system and the
///   stylesheet is compiled against them (`fonts::tick`);
/// * smaller copies of images drawn well below their size that finished on
///   the resampling thread are taken in, and the ones evicted go back to the
///   atlases (`resample::tick`).
pub fn frame_tick(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    // The pump's, not an input listener's: a layout read in one of these
    // handlers (an `<img>` `load` measuring the picture) may force a frame.
    off_input(|| frame_tick_inner(window, cx))
}

fn frame_tick_inner(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    // gpui's asset cache lets go of blob: decodes no node shows any more.
    img::evict_pending(window, cx);
    render::apply_pending_blur(window, cx);
    render::apply_pending_focus(window, cx);
    render::ensure_root_focus(window, cx);
    // `transitionend` / `animationstart` / `animationend` / `animationcancel`:
    // the animator queues them from the restyle pass and the paint clock, and
    // this is the one per-frame hook that has a `Window` to dispatch with.
    tree::advance_animations();
    // All of them, unconditionally: `|` rather than `||`, because a `<video>`
    // frame must be pulled even on a tick where a `<lottie>` also moved, and an
    // animation event must be dispatched even on a tick where both did.
    let anim = render::dispatch_anim_events(window, cx);
    // `<img>` `load` / `error`, for every `src` written since the last tick.
    img::tick(window, cx);
    let lottie_moved = lottie::tick(window, cx);
    let video_moved = video::tick(window, cx);
    // A `<canvas>` that drew since the last frame repaints, as a mutation
    // would; an idle one costs this one atomic swap (render::canvas).
    let canvas_moved = render::canvas::tick();
    // `el.scrollTop = n`, `scrollTo`, `scrollBy`, `scrollIntoView()`: the write
    // itself happened synchronously inside the host fn (a `ScrollHandle` is an
    // `Rc`, so the getter sees it on the next line), but the DOM `scroll` event
    // is asynchronous in a browser and asynchronous here — this is the frame
    // hook that fires it, and the `true` is what repaints a scroll no
    // `@scroll` listener was watching.
    let scrolled = scroll::dispatch_pending(window, cx);
    // ResizeObserver and IntersectionObserver, parked until a frame is built.
    wake_layout_watchers(window, cx);
    // Web fonts: loads started, finished ones registered with the text system
    // (and the sheet recompiled against them), `FontFace` statuses delivered.
    let fonts_moved = fonts::tick(window, cx);
    // Image copies for downscaled draws (#22): the ones that landed repaint.
    let resampled = resample::tick(window, cx);
    anim | lottie_moved | video_moved | canvas_moved | scrolled | fonts_moved | resampled
}

/// The per-frame work a SECONDARY window has to do for itself, because the
/// resource it touches belongs to that window and to no other
/// (docs/MULTIWINDOW.md §5.1).
///
/// Called from a pop-out's own `render()`, which is the only place that holds
/// its `&mut Window`. Three jobs, and each is a gpui resource the pump's window
/// physically cannot reach (plus the `blob:` decodes of [`frame_tick`], which
/// any window can hand back):
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
    img::evict_pending(window, cx);
    lottie::drain_pending_drops_for(window, cx);
    video::drain_pending_drops_for(window, cx);
    video::drain_pending_uploads(window);
    render::apply_pending_blur(window, cx);
    render::apply_pending_focus(window, cx);
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

    /// m26 client sweep #19. `[class*="group/msg"]` matched nothing: the class
    /// rides `set_class`, and attribute selectors read only the property map.
    #[test]
    fn attribute_selectors_see_the_class_attribute() {
        let (root, child) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let child = t.create_element(div);
            t.insert(child, root, 0);
            t.set_class(child, "row group/msg  hover:x");
            (root, child)
        });
        let q = |sel: &str| dom::query(sel, root, dom::QueryMode::Descendants).unwrap();
        assert_eq!(q(r#"[class*="group/msg"]"#), vec![child]);
        assert_eq!(q(r#"[class~="row"]"#), vec![child]);
        assert_eq!(q(r#"[class^="row "]"#), vec![child]);
        assert_eq!(q("[class]"), vec![child]);
        assert_eq!(q(".row"), vec![child], "the token list is unchanged");
        tree::with_tree(|t| t.set_class(child, ""));
        assert!(q("[class]").is_empty());
    }

    fn mounted_child() -> u32 {
        tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let child = t.create_element(div);
            t.insert(child, root, 0);
            child
        })
    }

    /// m26 client sweep #8. A node with no rectangle while the tree owes a
    /// layout, and no host that can draw (none here, as under a gpui input
    /// listener), reads STALE, and the runtime advises. It stays stale after the
    /// wave's flush took the changes, until a build lays them out; a node
    /// nothing is owed for is plainly "not painted".
    #[test]
    fn an_owed_layout_that_cannot_be_forced_reads_stale() {
        let child = mounted_child();
        assert_eq!(dom::bounds(child), (None, true));
        assert!(flush_changes());
        assert_eq!(dom::bounds(child), (None, true), "flushed is not laid out");
        render::note_build();
        assert_eq!(dom::bounds(child), (None, false), "built and still unpainted: not painted");
    }

    /// The host's hook is installed but refuses to draw (`in_input_dispatch`):
    /// the forced layout has flushed the changes for its draw, so the read is
    /// stale, and the wave's own flush must still report them, or the frame
    /// would not repaint (and pop-outs would not hear of it). A hook that
    /// draws makes the answer fresh.
    #[test]
    fn a_refused_forced_layout_keeps_the_waves_repaint() {
        fn refuse() {}
        set_sync_layout(refuse);
        let child = mounted_child();
        assert_eq!(dom::bounds(child), (None, true));
        assert!(!tree::read_tree(|t| t.is_dirty()), "the forced layout flushed");
        assert!(flush_changes(), "…and the wave still repaints");
        assert!(!flush_changes(), "once");

        fn draw() {
            render::note_build();
        }
        set_sync_layout(draw);
        let other = mounted_child();
        assert_eq!(dom::bounds(other), (None, false));
        assert!(flush_changes(), "a forced layout that drew still repaints every window");
    }

    /// m26 client sweep #27. A parked runtime is woken once, by the first
    /// frame tick after a build, and not again until it parks again: an idle
    /// window runs no observer JS at all.
    #[test]
    fn a_parked_layout_watch_wakes_once_after_a_build() {
        assert!(!take_layout_watch(), "nothing parked");
        watch_layout();
        assert!(!take_layout_watch(), "no build yet");
        assert!(!take_layout_watch(), "still none");
        render::note_build();
        assert!(take_layout_watch(), "a build wakes it");
        assert!(!take_layout_watch(), "once");
        render::note_build();
        assert!(!take_layout_watch(), "not parked again, so not woken again");
    }

    /// What may draw: a dispatch under `off_input` (the pump, a deferred wave)
    /// does not count as input, but it never hides an input dispatch around
    /// it, and gpui's draw (focus listeners) counts whatever encloses it.
    #[test]
    fn off_input_never_masks_an_enclosing_input_dispatch() {
        assert!(!in_input_dispatch());
        off_input(|| {
            let _d = InputDispatch::enter();
            assert!(!in_input_dispatch());
        });
        {
            let _d = InputDispatch::enter();
            assert!(in_input_dispatch());
            off_input(|| {
                let _n = InputDispatch::enter();
                assert!(in_input_dispatch());
            });
            assert!(in_input_dispatch());
        }
        assert!(!in_input_dispatch());
        off_input(|| {
            let _f = InputDispatch::inside_gpui();
            assert!(in_input_dispatch());
        });
        assert!(!in_input_dispatch());
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
/// runs its microtask checkpoint, so Vue's reactive flush lands in the same
/// frame. Must be called with `Phase::Commit` published. Returns `false` when
/// a JS handler called `preventDefault()`.
///
/// Under a gpui listener ([`in_input_dispatch`]) the checkpoint is OWED
/// rather than run (`vn_runtime::bridge::owe_drain`), and the listener's
/// wrapper queues it for after gpui's dispatch (`render::settle_owed_later`),
/// where an `onMounted` that measures can lay out. Any JS that starts before
/// then settles it first, so the order of checkpoints is unchanged.
pub fn dispatch_event(node: u32, event: u32, payload: &EventPayload) -> bool {
    dispatch_event_with(node, event, payload, None)
}

/// [`dispatch_event`] with an object for the event to carry, passed as the
/// dispatcher's eighth argument: a ClipboardEvent's `clipboardData`
/// (`crate::clipboard`). The caller keeps `data` alive across the call.
pub fn dispatch_event_with(node: u32, event: u32, payload: &EventPayload, data: Option<Value>) -> bool {
    let f = tree::dispatcher();
    if f == 0 {
        return true;
    }
    let span = perf::Span::start();
    let proceed = dispatch_event_inner(f, node, event, payload, data);
    perf::note_js(span.ms());
    proceed
}

fn dispatch_event_inner(f: u64, node: u32, event: u32, payload: &EventPayload, data: Option<Value>) -> bool {
    // The dispatch before this one may have left its checkpoint owed: it runs
    // first, as a browser's would have between the two events.
    vn_runtime::bridge::settle_owed_drain();
    // Under a gpui listener, owe this dispatch's checkpoint (see above). Before
    // the call: owing holds the engine lock, or JSC drains when the call ends.
    // A listener a forced layout's draw ran from inside a script (a focus
    // observer) is nested: `owe_drain` owes nothing there, and nothing drains
    // here either, because the script's own checkpoint runs these microtasks.
    let owe = in_input_dispatch();
    if owe {
        vn_runtime::bridge::owe_drain();
    }
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
            data.unwrap_or_else(Value::undefined),
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
        // Vue schedules its patch flush on Promise microtasks. Off a gpui
        // listener, drain now so the DOM ops run inside this Commit
        // publication; under one, the checkpoint stays owed.
        if !owe {
            rt.ctx().drain_microtasks();
        }
        proceed
    })
}
