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
mod color;
mod dom;
mod font_api;
mod fonts;
mod filter;
mod fullscreen;
mod img;
mod live;
mod live_state;
pub mod log;
mod lottie;
mod observe;
pub mod perf;
mod pointer;
mod range;
mod render;
mod resample;
mod scroll;
mod select;
mod selection;
mod svg;
mod tree;
mod video;
mod waapi;
pub mod windows;

pub mod devtools;

pub use observe::update_layout_observers;
pub use img::{asset_base, asset_store, content_key, dev_asset_file, set_asset_base, set_asset_store, set_dev_asset_files, AssetLookup};
pub use render::{
    bounds_in, build_root, build_subtree, control_hitboxes, control_hitboxes_in,
    enable_pick_hitboxes, flush_pending_mouse_move, painted_bounds_in, painted_drag_region,
    painted_drag_region_in, popped_out, root_bounds, root_bounds_in, set_os_chrome,
    set_os_chrome_in, set_popped_out, set_repaint_all,
};
pub use windows::WinKey;
pub use tree::{
    with_tree, EV_AUXCLICK, EV_BEFOREINPUT, EV_BLUR, EV_CANPLAY, EV_CANPLAYTHROUGH, EV_CLICK, EV_COMPLETE,
    EV_COMPOSITIONEND, EV_COMPOSITIONSTART, EV_COMPOSITIONUPDATE, EV_CONTEXTMENU,
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
    perf::note_dom(perf::DomOp::Create);
    ret_id(tree::with_tree(|t| t.create_element(tag)))
});

host_fn!(fn vn_create_text(_g, _args) {
    perf::note_dom(perf::DomOp::Create);
    ret_id(tree::with_tree(|t| t.create_text()))
});

host_fn!(fn vn_create_anchor(_g, _args) {
    perf::note_dom(perf::DomOp::Create);
    ret_id(tree::with_tree(|t| t.create_anchor()))
});

host_fn!(fn vn_set_text(g, args) {
    let id = arg_id(args.arg(0));
    let text = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Text);
    tree::with_tree(|t| t.set_text(id, text));
    UNDEF
});

host_fn!(fn vn_set_element_text(g, args) {
    let id = arg_id(args.arg(0));
    let text = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Text);
    tree::with_tree(|t| t.set_element_text(id, text));
    UNDEF
});

host_fn!(fn vn_insert(_g, args) {
    let (child, parent, anchor) = (arg_id(args.arg(0)), arg_id(args.arg(1)), arg_id(args.arg(2)));
    perf::note_dom(perf::DomOp::Insert);
    tree::with_tree(|t| t.insert(child, parent, anchor));
    UNDEF
});

host_fn!(fn vn_remove(_g, args) {
    let id = arg_id(args.arg(0));
    perf::note_dom(perf::DomOp::Remove);
    tree::with_tree(|t| t.remove(id));
    UNDEF
});

host_fn!(fn vn_set_style(g, args) {
    let id = arg_id(args.arg(0));
    let key = arg_id(args.arg(1));
    let value = unsafe { read_string(g, args.arg(2).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Style);
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
    perf::note_dom(perf::DomOp::Class);
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
    perf::note_dom(perf::DomOp::Prop);
    let img_src = tree::with_tree(|t| {
        t.set_prop(id, key, value);
        t.name(key) == "src" && t.node(id).is_some_and(|n| t.name(n.tag) == "img")
    });
    // An `<img>`'s `src` is answered with `load` / `error` (`img::tick`),
    // whether or not the element is ever painted.
    if img_src {
        perf::note_img_src();
        img::note_src(id);
    }
    UNDEF
});

host_fn!(fn vn_add_scope(g, args) {
    let id = arg_id(args.arg(0));
    let scope = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Other);
    tree::with_tree(|t| t.add_scope(id, scope));
    UNDEF
});

host_fn!(fn vn_set_stylesheet(g, args) {
    let css = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    // `document.head`'s injected `<style>` texts, each on its own (runtime.ts).
    // A runtime that predates the argument passes none, and its concatenation
    // in `css` compiles exactly as before.
    let extra = args.arg(1).0;
    let head: Vec<String> = if vn_runtime::redjs::webapi::util::is_cell(extra) {
        unsafe {
            use vn_runtime::redjs::webapi::util::{array_len, get_i};
            (0..array_len(g, extra)).filter_map(|i| read_string(g, get_i(g, extra, i))).collect()
        }
    } else {
        Vec::new()
    };
    // Every node needs a fresh cascade under a new sheet — and under the one
    // already installed, re-sent unchanged, nothing does.
    if tree::set_stylesheet_with_head(css, head) {
        perf::note_full(perf::FullReason::Stylesheet);
        tree::with_tree(|t| t.mark_dirty());
    }
    UNDEF
});

host_fn!(fn vn_set_listener(g, args) {
    let id = arg_id(args.arg(0));
    let event = arg_id(args.arg(1));
    let on = args.arg(2).as_bool().unwrap_or(false);
    let _ = g;
    perf::note_dom(perf::DomOp::Other);
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
    perf::note_read(perf::Read::Query);
    let mode = match arg_id(args.arg(2)) {
        1 => dom::QueryMode::SelfOnly,
        2 => dom::QueryMode::Closest,
        3 => dom::QueryMode::Document,
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
    perf::note_read(perf::Read::Layout);
    let out = match dom::bounds(id) {
        // The layout box, then — when a CSS `transform` moved the picture —
        // where it was drawn, which is getBoundingClientRect()'s answer.
        (Some((x, y, w, h)), _) => match dom::transformed(id) {
            Some((tx, ty, tw, th)) => format!("{x},{y},{w},{h},{tx},{ty},{tw},{th}"),
            None => format!("{x},{y},{w},{h}"),
        },
        (None, true) => "stale".to_string(),
        (None, false) => String::new(),
    };
    ret_string(&out)
});

// `getComputedStyle(el)` — `name=value` lines; see `dom::computed_style`.
host_fn!(fn vn_computed_style(_g, args) {
    let id = arg_id(args.arg(0));
    perf::note_read(perf::Read::Style);
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
    perf::note_read(perf::Read::Scroll);
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
//         events) | "refused" (not focusable, not in the document, or hidden
//         by CSS: `display: none`, or a computed `visibility` other than
//         `visible`)
//   op 1  `el.blur()`  -> "blurred" (JS fires blur/focusout) | ""
//
// Outside a Commit publication (no window: module-scope code at startup), a
// focus is queued and a blur is a no-op.
host_fn!(fn vn_focus(_g, args) {
    let id = arg_id(args.arg(0));
    let op = arg_id(args.arg(1));
    // A browser flushes style and refuses an element CSS hides before anything
    // else, so there is nothing to build or queue for one
    // (render::style_refuses_focus, docs/RUNTIME.md §5.20).
    if op == 0 && render::style_refuses_focus(id) {
        return ret_string("refused");
    }
    // A node mounted in this same update has no handle yet: lay out first
    // where that is legal, so the focus is synchronous (render::focus_wants_build).
    // Under a gpui listener this draws nothing and the focus is queued (FO-1).
    if op == 0 && layout_owed() && render::focus_wants_build(id) {
        sync_layout_by(perf::SyncCaller::Focus);
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

// Select the native text field, or queue it until its first native build.
host_fn!(fn vn_input_select(_g, args) {
    Value::boolean(render::select_input_text(arg_id(args.arg(0)))).0
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

// One scalar media property by code (`video::media_prop`): a number, or
// `undefined` for a node that is not a live media element.
host_fn!(fn vn_media_prop(_g, args) {
    let id = arg_id(args.arg(0));
    let code = arg_id(args.arg(1));
    video::media_prop(id, code).map_or(UNDEF, |v| Value::number(v).0)
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

host_fn!(fn vn_fullscreen_element(_g, args) {
    Value::boolean(fullscreen::set(arg_id(args.arg(0)))).0
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
        ("__vn_fullscreen_element", 1, vn_fullscreen_element as *const _),
        ("__vn_set_dispatcher", 1, vn_set_dispatcher as *const _),
        ("__vn_query", 3, vn_query as *const _),
        ("__vn_bounds", 1, vn_bounds as *const _),
        ("__vn_computed_style", 1, vn_computed_style as *const _),
        ("__vn_scroll", 4, vn_scroll as *const _),
        ("__vn_scroll_into_view", 3, vn_scroll_into_view as *const _),
        ("__vn_focus", 2, vn_focus as *const _),
        ("__vn_input_select", 1, vn_input_select as *const _),
        ("__vn_media_state", 1, vn_media_state as *const _),
        ("__vn_media_prop", 2, vn_media_prop as *const _),
        ("__vn_media_call", 3, vn_media_call as *const _),
        ("__vn_media_can_play", 2, vn_media_can_play as *const _),
        ("__vn_media_events", 1, vn_media_events as *const _),
        ("__vn_img_bitmap", 2, vn_img_bitmap as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
    // Direct native Web Animations normalization, timing and painter methods.
    waapi::install(rt);
    // `getSelection()` / `Range` (crate::selection).
    selection::install(rt);
    // `document.elementFromPoint` / `elementsFromPoint` (crate::pointer).
    pointer::install(rt);
    // ResizeObserver and IntersectionObserver after layout (crate::observe).
    observe::install(rt);
    // Native FontFace/FontFaceSet objects with a thin document attachment.
    font_api::install(rt);
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
    sync_layout_inner();
}

/// [`sync_layout`], counted for the devtools frame trace by the DOM read that
/// asked: whether it drew (a build happened) and the wall time of the whole
/// call — the restyle it flushes and gpui's full window draw.
pub(crate) fn sync_layout_by(caller: perf::SyncCaller) {
    if IN_SYNC_LAYOUT.with(|c| c.get()) {
        return;
    }
    if !perf::enabled() {
        sync_layout_inner();
        return;
    }
    let before = render::build_count();
    let span = perf::Span::start();
    sync_layout_inner();
    perf::note_sync_layout(caller, render::build_count() != before, span.ms());
}

fn sync_layout_inner() {
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

/// What a JS wave left for the frame ([`flush_changes_kind`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flush {
    /// The tree did not change.
    Unchanged,
    /// The tree changed, and no element-tree build has laid it out since.
    Changed,
    /// Every change of the wave was flushed by a forced layout, and that
    /// layout's draw came after the last of them: the primary window already
    /// holds a drawn frame of the tree as it stands, waiting to be presented.
    DrawnBySync,
}

/// Call after a JS wave: if the tree changed, run the restyle pass (inline
/// parse + cascade) and report `true` so the caller notifies the root view.
pub fn flush_changes() -> bool {
    flush_changes_kind() != Flush::Unchanged
}

/// [`flush_changes`], telling apart the wave whose changes a forced layout has
/// already drawn ([`Flush::DrawnBySync`]) from one that still needs a draw.
pub fn flush_changes_kind() -> Flush {
    let dirty = tree::with_tree(|t| t.take_dirty());
    if dirty {
        tree::restyle_all();
        render::gc_node_state();
        render::canvas::gc(|id| tree::read_tree(|t| t.node(id).is_some()));
        tree::animator_gc();
        // Only now may this wave's freed ids be reused (`Tree::release_freed`).
        tree::with_tree(|t| t.release_freed());
        FLUSHED_AT_BUILD.with(|c| c.set(Some(render::build_count())));
    }
    // Inside a forced layout this is its own flush; outside, it also answers
    // for the changes a forced layout took earlier in the wave.
    let forced = !IN_SYNC_LAYOUT.with(|c| c.get()) && FLUSHED_BY_SYNC.with(|c| c.replace(false));
    if dirty {
        Flush::Changed
    } else if !forced {
        Flush::Unchanged
    } else if layout_owed() {
        // Flushed by a forced layout the host could not draw.
        Flush::Changed
    } else {
        Flush::DrawnBySync
    }
}

/// True while any CSS transition runs — the host keeps repainting.
pub fn animating() -> bool {
    tree::animating()
}

/// True while any `<lottie>` still needs frames (crates/vn-lottie/CONTRACT.md
/// §4), or a load or a raster is out on its worker.
///
/// Not a reason to repaint (§8.10): the pump ticks every frame anyway, and
/// [`frame_tick`] writes a `<lottie>`'s frames into its tile, asking for a
/// draw only when a present cannot show them.
pub fn lotties_animating() -> bool {
    lottie::any_playing()
}

/// True while any `<video>` still needs frames (playing, seeking, or waiting
/// for its first frame).
///
/// Not a reason to repaint: the pump ticks every video every frame anyway, and
/// [`frame_tick`] reports a decoded frame when the video can be on screen. The
/// host used to OR this into its repaint condition, which redrew the window
/// at the display rate for a video nobody could see.
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
/// * every `<vn-sticker>` source is fetched and asked for at its boxes' sizes,
///   and the frames due are written into this window's tiles
///   (`render::sticker`);
/// * a `<canvas>` that drew since the last frame asks for a repaint;
/// * a `<video>` or moving `<canvas>` frame written into the atlas tile the
///   last drawn scene samples asks only for a present (`crate::live`): the
///   window shows it with no build, and this returns `false` for it;
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
    // A script animation created, moved or dropped since the last tick
    // (`Element.animate()`): a seek on a paused one is not "animating", and
    // nothing else would draw it.
    let scripted = tree::take_script_dirty();
    // `<img>` `load` / `error`, for every `src` written since the last tick.
    img::tick(window, cx);
    let lottie_moved = lottie::tick(window, cx);
    let video_moved = video::tick(window, cx);
    // `<vn-sticker>`: fetches, targets, this window's tiles written, events.
    let stickers_moved = render::sticker::tick(window, cx);
    // A `<canvas>` that drew since the last frame repaints, as a mutation
    // would; an idle one costs this one atomic swap (render::canvas).
    let canvas_moved = render::canvas::tick(window);
    // `el.scrollTop = n`, `scrollTo`, `scrollBy`, `scrollIntoView()`: the write
    // itself happened synchronously inside the host fn (a `ScrollHandle` is an
    // `Rc`, so the getter sees it on the next line), but the DOM `scroll` event
    // is asynchronous in a browser and asynchronous here — this is the frame
    // hook that fires it, and the `true` is what repaints a scroll no
    // `@scroll` listener was watching.
    let scrolled = scroll::dispatch_pending(window, cx);
    // Web fonts: loads started, finished ones registered with the text system
    // (and the sheet recompiled against them), `FontFace` statuses delivered.
    let fonts_moved = fonts::tick(window, cx);
    // Image copies for downscaled draws (#22): the ones that landed repaint.
    let resampled = resample::tick(window, cx);
    // What gpui's asset cache keeps of the images: decodes nobody shows go,
    // decodes drawn only small give way to their copy, and come back full
    // when drawn large (`img::residency_tick`). After the resample tick: it
    // takes the copies that tick kept.
    let restored = img::residency_tick(window, cx);
    // A text selection being dragged: what waited for this frame resolves, a
    // drag past its scroll container's edge scrolls it, and an owed
    // `selectionchange` goes to `document`. One flag read otherwise.
    let selecting = selection::tick(window, cx);
    let draw =
        anim | scripted | lottie_moved | video_moved | stickers_moved | canvas_moved | scrolled | fonts_moved
            | resampled | restored | selecting;
    // A frame written into the tile the last drawn scene samples is shown by a
    // present, with no build (`crate::live`); one that needs a draw is folded
    // into the answer, and a frame that draws anyway presents what it drew.
    match live::take() {
        live::Shown::Draw => true,
        live::Shown::Present if !draw => {
            live::present(window);
            false
        }
        _ => draw,
    }
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
///   window's atlas, or the element paints transparent here, and
///   **`<vn-sticker>` frames** are written into THIS window's tiles;
/// * **focus**: a node detached while focused in THIS window loses gpui focus,
///   and this window keeps something on its own focus path so keystrokes have
///   somewhere to travel.
///
/// The JS-facing half of `frame_tick` — the lottie clock, the `<video>`
/// transport, the CSS animation clock — is deliberately NOT here: it is a
/// property of the one JS realm and must run once per FRAME GROUP, not once per
/// window (D-M06). Running it twice would double every `setTimeout` rate.
pub fn window_frame_tick(window: &mut gpui::Window, cx: &mut gpui::App) {
    pointer::arm_hover_tick(window);
    img::evict_pending(window, cx);
    lottie::window_tick(window, cx);
    video::drain_pending_drops_for(window, cx);
    video::drain_pending_uploads(window);
    render::sticker::window_tick(window);
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
    render::sticker::forget_window(key);
    lottie::forget_window(key);
}

/// The page became hidden or shown (minimised, or on another virtual desktop):
/// what `document.hidden` reports. A paint-only frame is not presented while
/// the page is hidden (`crate::live`); the draw that shows the window again
/// paints the newest one.
pub fn set_page_hidden(hidden: bool) {
    live::set_hidden(hidden);
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
        perf::note_full(perf::FullReason::Env);
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
        assert_eq!(flush_changes_kind(), Flush::Changed, "…and the wave still draws, as nothing did");
        assert!(!flush_changes(), "once");

        fn draw() {
            render::note_build();
        }
        set_sync_layout(draw);
        let other = mounted_child();
        assert_eq!(dom::bounds(other), (None, false));
        assert_eq!(
            flush_changes_kind(),
            Flush::DrawnBySync,
            "a forced layout that drew still reports the change (pop-outs repaint), as already drawn",
        );
        assert_eq!(flush_changes_kind(), Flush::Unchanged, "once");

        // A change after the forced layout's draw is the frame's to draw.
        let third = mounted_child();
        assert_eq!(dom::bounds(third), (None, false));
        tree::with_tree(|t| t.set_class(third, "moved"));
        assert_eq!(flush_changes_kind(), Flush::Changed);
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

    /// An infinite animation stops with its node, even when Vue hands the
    /// node's id to a new one in the same wave. The Argon client's dashboard
    /// spinner did exactly that on every space open, and the window then
    /// repainted at the display's rate for good.
    #[test]
    fn an_infinite_animation_dies_with_its_node_when_the_wave_reuses_nothing() {
        tree::set_engine(vn_css::Engine::compile(
            ".spin { animation: spin 1s linear infinite } @keyframes spin { to { opacity: 0.5 } }",
        ));
        let spinner = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let spinner = t.create_element(div);
            t.set_class(spinner, "spin");
            t.insert(spinner, root, 0);
            spinner
        });
        assert!(flush_changes());
        assert!(animating(), "the spinner runs");

        // Removed, and a Vue anchor created in the same wave.
        let anchor = tree::with_tree(|t| {
            t.remove(spinner);
            t.create_anchor()
        });
        assert_ne!(anchor, spinner, "a freed id is not reused before the sweep");
        flush_changes();
        assert!(!animating(), "nothing is left running");
        let reused = tree::with_tree(|t| t.create_anchor());
        assert_eq!(reused, spinner, "after the sweep it is");
    }

    /// CSS Animations §3: an element that is `display: none`, or under one,
    /// runs no animation. Hiding cancels its runs (with `animationcancel`) and
    /// those of everything under it, even nodes whose own style did not move;
    /// showing starts them afresh; a node inserted under a hidden one never
    /// starts.
    #[test]
    fn display_none_stops_the_animations_under_it_and_showing_restarts_them() {
        tree::set_engine(vn_css::Engine::compile(
            ".spin { animation: spin 1s linear infinite } .gone { display: none } \
             @keyframes spin { to { opacity: 0.5 } }",
        ));
        let (panel, spinner) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let panel = t.create_element(div);
            let spinner = t.create_element(div);
            t.set_class(spinner, "spin");
            t.insert(panel, root, 0);
            t.insert(spinner, panel, 0);
            (panel, spinner)
        });
        flush_changes();
        tree::advance_animations();
        tree::take_anim_events();
        assert!(animating(), "the spinner runs");

        // The PANEL is hidden; the spinner's own style does not change.
        tree::with_tree(|t| t.set_class(panel, "gone"));
        flush_changes();
        assert!(!animating(), "nothing runs under display: none");
        let cancelled: Vec<_> = tree::take_anim_events()
            .into_iter()
            .filter(|e| e.node == spinner && e.kind == vn_css::AnimEventKind::AnimationCancel)
            .collect();
        assert_eq!(cancelled.len(), 1, "one animationcancel for the spinner");

        // Inserted under the hidden panel: never starts.
        let late = tree::with_tree(|t| {
            let div = t.intern("div");
            let late = t.create_element(div);
            t.set_class(late, "spin");
            t.insert(late, panel, 1);
            late
        });
        flush_changes();
        assert!(!animating(), "a node mounted under display: none does not start ({late})");

        // Shown again: both start.
        tree::with_tree(|t| t.set_class(panel, ""));
        flush_changes();
        assert!(animating(), "showing the panel restarts what is under it");
        let mut hot = std::collections::HashSet::new();
        tree::moving_paths(std::time::Instant::now(), &mut hot);
        assert!(hot.contains(&spinner) && hot.contains(&late));

        // The element itself hidden: the same.
        tree::with_tree(|t| t.set_class(spinner, "spin gone"));
        flush_changes();
        hot.clear();
        tree::moving_paths(std::time::Instant::now(), &mut hot);
        assert!(!hot.contains(&spinner) && hot.contains(&late));
    }

    /// The retained-render cache refuses only the boundaries on a moving node's
    /// path: the node and its ancestors, never a sibling subtree. A paused run
    /// is not moving (its frame is frozen), and neither is a node whose
    /// animation has not been declared.
    #[test]
    fn only_a_moving_nodes_ancestors_are_hot() {
        tree::set_engine(vn_css::Engine::compile(
            ".spin { animation: spin 1s linear infinite } .held { animation-play-state: paused } \
             @keyframes spin { to { opacity: 0.5 } }",
        ));
        let (list, row, spinner, other, held) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let list = t.create_element(div);
            let row = t.create_element(div);
            let spinner = t.create_element(div);
            let other = t.create_element(div);
            let held = t.create_element(div);
            t.set_class(spinner, "spin");
            t.set_class(held, "spin held");
            t.insert(list, root, 0);
            t.insert(row, list, 0);
            t.insert(spinner, row, 0);
            t.insert(other, list, 1);
            t.insert(held, other, 0);
            (list, row, spinner, other, held)
        });
        flush_changes();
        let mut hot = std::collections::HashSet::new();
        tree::moving_paths(std::time::Instant::now(), &mut hot);
        let root = tree::read_tree(|t| t.root);
        for id in [spinner, row, list, root] {
            assert!(hot.contains(&id), "{id} is on the spinner's path");
        }
        assert!(!hot.contains(&other), "a sibling subtree stays replayable");
        assert!(!hot.contains(&held), "a paused animation is not moving");

        // Stopped: nothing is hot, so every boundary may replay again.
        tree::with_tree(|t| t.set_class(spinner, ""));
        flush_changes();
        hot.clear();
        tree::moving_paths(std::time::Instant::now(), &mut hot);
        assert!(hot.is_empty(), "nothing moves: {hot:?}");
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

    /// `.stack > *` is offered to an element only when its parent carries the
    /// key (vn-css's parent buckets), so no child of a plain `div` ever asks the
    /// parent about `stack`. The change still has to reach the children.
    #[test]
    fn a_parent_gaining_the_key_of_a_parent_keyed_rule_restyles_its_children() {
        let (mid, first, second) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mid = t.create_element(div);
            let first = t.create_element(div);
            let second = t.create_element(div);
            t.insert(mid, root, 0);
            t.insert(first, mid, 0);
            t.insert(second, mid, 1);
            (mid, first, second)
        });
        tree::set_engine(vn_css::Engine::compile(
            "* { opacity: 0.1 } .stack > * { opacity: 0.5 } .space > * + * { opacity: 0.9 } \
             #list > * { opacity: 0.3 }",
        ));
        tree::restyle_all();
        assert_eq!((opacity_of(first), opacity_of(second)), (Some(0.1), Some(0.1)));

        tree::with_tree(|t| t.set_class(mid, "stack"));
        flush_changes();
        assert_eq!((opacity_of(first), opacity_of(second)), (Some(0.5), Some(0.5)));

        tree::with_tree(|t| t.set_class(mid, "space"));
        flush_changes();
        assert_eq!((opacity_of(first), opacity_of(second)), (Some(0.1), Some(0.9)));

        tree::with_tree(|t| {
            t.set_class(mid, "");
            let key = t.intern("id");
            t.set_prop(mid, key, Some("list".into()));
        });
        flush_changes();
        assert_eq!((opacity_of(first), opacity_of(second)), (Some(0.3), Some(0.3)));
    }

    /// The class list is a set, as `classList` is: the rule index offers a
    /// class's rules once per entry, so a repeated class would run them twice.
    /// The attribute keeps the string as written.
    #[test]
    fn a_repeated_class_is_one_entry_of_the_class_list() {
        tree::with_tree(|t| {
            let root = t.create_root();
            let tag = t.intern("svg");
            let svg = t.create_element(tag);
            t.insert(svg, root, 0);
            t.set_class(svg, "lucide w-4 h-4 lucide-plus w-4 h-4");
            let n = t.node(svg).unwrap();
            assert_eq!(n.classes, ["lucide", "w-4", "h-4", "lucide-plus"]);
            assert_eq!(n.class_attr, "lucide w-4 h-4 lucide-plus w-4 h-4");
        });
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
    /// recascade the siblings around it. The cascade recorded that `only`
    /// asked for its next sibling (`tree::sel::NEXT_EDGE`), and that is what
    /// the insert marks; it no longer marks the whole parent.
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

    /// `:nth-child()` counts every earlier sibling, so an insert at the front
    /// moves the index of everything after it, including elements far from
    /// the insert. Removing it again moves them back.
    #[test]
    fn a_prepend_moves_the_nth_child_answer_of_every_later_sibling() {
        let (parent, items) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let parent = t.create_element(div);
            t.insert(parent, root, 0);
            let items: Vec<u32> = (0..4)
                .map(|_| {
                    let id = t.create_element(div);
                    t.insert(id, parent, 0);
                    t.set_class(id, "item");
                    id
                })
                .collect();
            (parent, items)
        });
        tree::set_engine(vn_css::Engine::compile(
            ".item { opacity: 0.3 } .item:nth-child(4) { opacity: 0.7 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(items[3]), Some(0.7));

        let first = tree::with_tree(|t| {
            let div = t.intern("div");
            let first = t.create_element(div);
            t.insert(first, parent, items[0]);
            first
        });
        flush_changes();
        assert_eq!(opacity_of(items[2]), Some(0.7), "third item is the fourth child now");
        assert_eq!(opacity_of(items[3]), Some(0.3), "and the last one the fifth");

        tree::with_tree(|t| t.remove(first));
        flush_changes();
        assert_eq!(opacity_of(items[2]), Some(0.3));
        assert_eq!(opacity_of(items[3]), Some(0.7));
    }

    /// `:empty` is a question about the child list too: the element that was
    /// matched against it is restyled when an insert or a remove flips the
    /// answer, and a text child gaining its first character flips it as well.
    #[test]
    fn empty_follows_inserts_removes_and_text() {
        let boxed = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let boxed = t.create_element(div);
            t.insert(boxed, root, 0);
            t.set_class(boxed, "box");
            boxed
        });
        tree::set_engine(vn_css::Engine::compile(".box { opacity: 0.3 } .box:empty { opacity: 0.7 }"));
        tree::restyle_all();
        assert_eq!(opacity_of(boxed), Some(0.7));

        let child = tree::with_tree(|t| {
            let span = t.intern("span");
            let child = t.create_element(span);
            t.insert(child, boxed, 0);
            child
        });
        flush_changes();
        assert_eq!(opacity_of(boxed), Some(0.3), "an element child");

        tree::with_tree(|t| t.remove(child));
        flush_changes();
        assert_eq!(opacity_of(boxed), Some(0.7), "removed again");

        let text = tree::with_tree(|t| {
            let text = t.create_text();
            t.insert(text, boxed, 0);
            text
        });
        flush_changes();
        assert_eq!(opacity_of(boxed), Some(0.7), "an empty text node is still empty");
        tree::with_tree(|t| t.set_text(text, "hi".into()));
        flush_changes();
        assert_eq!(opacity_of(boxed), Some(0.3), "text with data");
    }

    /// `:has()` reads DOWN, so a change anywhere under the anchor has to
    /// re-match it: a child inserted and removed, and an attribute written
    /// two levels below.
    #[test]
    fn has_follows_changes_anywhere_under_the_anchor() {
        let (card, inner) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let card = t.create_element(div);
            let inner = t.create_element(div);
            t.insert(card, root, 0);
            t.insert(inner, card, 0);
            t.set_class(card, "card");
            (card, inner)
        });
        tree::set_engine(vn_css::Engine::compile(
            ".card { opacity: 0.3 } .card:has(> .icon) { opacity: 0.6 } .card:has([data-on]) { opacity: 0.9 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(card), Some(0.3));

        let icon = tree::with_tree(|t| {
            let i = t.intern("i");
            let icon = t.create_element(i);
            t.set_class(icon, "icon");
            t.insert(icon, card, 0);
            icon
        });
        flush_changes();
        assert_eq!(opacity_of(card), Some(0.6), "a direct child arrived");

        tree::with_tree(|t| t.remove(icon));
        flush_changes();
        assert_eq!(opacity_of(card), Some(0.3), "and left");

        let deep = tree::with_tree(|t| {
            let span = t.intern("span");
            let deep = t.create_element(span);
            t.insert(deep, inner, 0);
            deep
        });
        flush_changes();
        let on = tree::with_tree(|t| t.intern("data-on"));
        tree::with_tree(|t| t.set_prop(deep, on, Some(String::new())));
        flush_changes();
        assert_eq!(opacity_of(card), Some(0.9), "an attribute two levels down");
    }

    /// A `:has()` further left in the selector makes a DESCENDANT's match read
    /// the anchor's subtree, so the change has to reach the descendant, not
    /// only the anchor.
    #[test]
    fn has_left_of_a_combinator_reaches_the_descendants_that_read_it() {
        let (panel, title, slot) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let panel = t.create_element(div);
            let title = t.create_element(div);
            let slot = t.create_element(div);
            t.insert(panel, root, 0);
            t.insert(title, panel, 0);
            t.insert(slot, panel, 0);
            t.set_class(panel, "panel");
            t.set_class(title, "title");
            (panel, title, slot)
        });
        tree::set_engine(vn_css::Engine::compile(
            ".title { opacity: 0.3 } .panel:has(.editor) .title { opacity: 0.8 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(title), Some(0.3));

        tree::with_tree(|t| {
            let div = t.intern("div");
            let editor = t.create_element(div);
            t.set_class(editor, "editor");
            t.insert(editor, slot, 0);
        });
        flush_changes();
        assert_eq!(opacity_of(title), Some(0.8), "the editor arrived next door to the title");
        let _ = panel;
    }

    /// A wave of hundreds of separate marks stays a scoped pass (there used to
    /// be a limit of 64, past which the whole document was walked instead), and
    /// every marked node still ends up restyled.
    #[test]
    fn a_wave_of_hundreds_of_separate_marks_restyles_every_one() {
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

        // 200 separate class writes, each its own walk root.
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

    /// `el.focus()` asks CSS what a browser asks (docs/RUNTIME.md §5.20): an
    /// element in a `display: none` subtree, or whose inherited `visibility`
    /// is not `visible`, refuses it. A screen-reader-only box paints nothing
    /// but is `visible`, and a `visible` declaration under a hidden box wins.
    #[test]
    fn focus_refuses_what_css_hides() {
        let ids = tree::with_tree(|t| {
            let root = t.create_root();
            let tag = t.intern("div");
            let mut kid = |parent: u32, class: &str| {
                let id = t.create_element(tag);
                t.insert(id, parent, 0);
                t.set_class(id, class);
                id
            };
            let hidden = kid(root, "hidden");
            let under_hidden = kid(hidden, "plain");
            let shown = kid(hidden, "shown");
            let gone = kid(root, "gone");
            let under_gone = kid(gone, "plain");
            let sr = kid(root, "sr");
            let plain = kid(root, "plain");
            [under_hidden, shown, under_gone, sr, plain]
        });
        tree::set_engine(vn_css::Engine::compile(
            ".hidden { visibility: hidden } .shown { visibility: visible } .gone { display: none } \
             .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0, 0, 0, 0) }",
        ));
        flush_changes();
        let [under_hidden, shown, under_gone, sr, plain] = ids;
        assert!(render::style_refuses_focus(under_hidden), "visibility inherits");
        assert!(!render::style_refuses_focus(shown), "the nearest declaration decides");
        assert!(render::style_refuses_focus(under_gone), "a display: none subtree is not rendered");
        assert!(!render::style_refuses_focus(sr), "a screen-reader-only box is visible to CSS");
        assert!(!render::style_refuses_focus(plain));
        assert!(tree::read_tree(|t| !render::css_visible(t, under_hidden) && render::css_visible(t, sr)));
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

thread_local! {
    /// The pointer position, window CSS px, for the next dispatch only.
    static POINTER_AT: std::cell::Cell<Option<(f64, f64)>> = const { std::cell::Cell::new(None) };
}

/// The next [`dispatch_event`] carries the pointer at (`x`, `y`) as the
/// dispatcher's ninth and tenth arguments: a `wheel` is a MouseEvent, and its
/// `clientX` / `clientY` are where it happened, which its payload has no room
/// for (its `a`/`b` are the deltas).
pub(crate) fn pointer_at_next_dispatch(x: f64, y: f64) {
    POINTER_AT.with(|c| c.set(Some((x, y))));
}

/// [`dispatch_event`] with an object for the event to carry, passed as the
/// dispatcher's eighth argument: a ClipboardEvent's `clipboardData`
/// (`crate::clipboard`). The caller keeps `data` alive across the call.
pub fn dispatch_event_with(node: u32, event: u32, payload: &EventPayload, data: Option<Value>) -> bool {
    // Taken first, so a position set for a dispatch that goes nowhere never
    // reaches the next one.
    let at = POINTER_AT.with(|c| c.take());
    let f = tree::dispatcher();
    if f == 0 {
        return true;
    }
    let span = perf::Span::start();
    let proceed = dispatch_event_inner(f, node, event, payload, data, at);
    perf::note_js(span.ms());
    proceed
}

fn dispatch_event_inner(f: u64, node: u32, event: u32, payload: &EventPayload, data: Option<Value>, at: Option<(f64, f64)>) -> bool {
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
    // `at`: where the pointer is, for an event that carries it beside its own
    // four numbers — a `wheel` (render.rs), whose `a`/`b` are its deltas.
    JsRuntime::with(|rt| {
        let key = payload.key.map(|k| rt.ctx().string(k));
        let (x, y) = at.map_or((Value::undefined(), Value::undefined()), |(x, y)| (Value::double(x), Value::double(y)));
        let args = [
            Value::int32(node as i32),
            Value::int32(event as i32),
            Value::double(payload.a),
            Value::double(payload.b),
            Value::double(payload.c),
            Value::double(payload.d),
            key.unwrap_or_else(Value::undefined),
            data.unwrap_or_else(Value::undefined),
            x,
            y,
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
