//! sk-tree — the retained node tree that Vue's custom renderer mutates through `__sk_*` host
//! functions, and the per-frame lowering of that tree into gpui elements.
//!
//! JS holds dense `u32` node ids (0 = none); tags, style keys and event names are interned once via
//! `__sk_intern` so the hot path carries no strings. Structural ops apply synchronously; `insert` is
//! also move (implicit detach), `remove` destroys the whole subtree. Mutations mark the tree dirty
//! and the frame pump notifies the root view after the JS wave.

mod aria;
mod bgimage;
mod cache;
mod clipboard;
mod color;
mod css_namespace;
mod cull;
mod dom;
mod effect_texture;
mod filter;
mod font_api;
mod fonts;
mod fullscreen;
mod geometry_js;
mod idmap;
pub mod iframe;
mod img;
mod inline_style;
mod live;
mod live_state;
pub mod log;
mod lottie;
mod mask;
mod native_dom;
mod observe;
pub mod perf;
mod pointer;
mod pointer_lock;
mod range;
mod render;
mod resample;
mod scroll;
mod select;
mod selection;
mod sticky;
mod style_decl;
pub mod surface;
mod svg;
mod toggle;
mod tree;
mod video;
mod view_transition;
mod view_transition_js;
mod waapi;
pub mod windows;

pub mod devtools;

pub use img::{
    asset_base, asset_store, content_key, dev_asset_file, read_app_asset, set_asset_base,
    set_asset_store, set_dev_asset_files, AssetLookup,
};
pub use observe::update_layout_observers;
pub use pointer_lock::{
    intercept_escape as pointer_lock_intercept_escape, navigated as pointer_lock_navigated,
};
pub use render::{
    bounds_in, build_root, build_subtree, control_hitboxes, control_hitboxes_in,
    enable_pick_hitboxes, flush_pending_mouse_move, painted_bounds_in, painted_drag_region,
    painted_drag_region_in, popped_out, root_bounds, root_bounds_in, set_os_chrome,
    set_os_chrome_in, set_popped_out, set_repaint_all, window_control_at,
};
pub use tree::{
    effects_changed, with_tree, EVENT_NAMES, EV_AUXCLICK, EV_BEFOREINPUT, EV_BLUR, EV_CANPLAY,
    EV_CANPLAYTHROUGH, EV_CLICK, EV_COMPLETE, EV_COMPOSITIONEND, EV_COMPOSITIONSTART,
    EV_COMPOSITIONUPDATE, EV_CONTEXTMENU, EV_COPY, EV_CUT, EV_DURATIONCHANGE, EV_EMPTIED, EV_ENDED,
    EV_ENTERFRAME, EV_ERROR, EV_FOCUS, EV_INPUT, EV_KEYDOWN, EV_KEYUP, EV_LOAD, EV_LOADEDDATA,
    EV_LOADEDMETADATA, EV_LOADSTART, EV_LOOPCOMPLETE, EV_MESSAGE, EV_MOUSEDOWN, EV_MOUSEENTER,
    EV_MOUSELEAVE, EV_MOUSEMOVE, EV_MOUSEUP, EV_PASTE, EV_PAUSE, EV_PLAY, EV_PLAYING,
    EV_POINTERCANCEL, EV_POINTERLOCKCHANGE, EV_POINTERLOCKERROR, EV_RATECHANGE, EV_SCROLL,
    EV_SEEKED, EV_SEEKING, EV_TIMEUPDATE, EV_VOLUMECHANGE, EV_WAITING, EV_WHEEL,
};
pub use view_transition::build_overlay as build_view_transition;
pub use windows::WinKey;

/// The numeric payload a native input event carries into JS. Mouse: a=x b=y c=button, d=modifier
/// bits | click count << 4. Wheel: a=deltaX b=deltaY. Input: `key` = the new value. Scroll:
/// a=scrollLeft b=scrollTop. Modifier bits: 1 ctrl, 2 shift, 4 alt, 8 meta.
#[derive(Default)]
pub struct EventPayload<'a> {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub key: Option<&'a str>,
}

use std::ffi::c_void;

use sk_runtime::redjs::api::Value;
use sk_runtime::redjs::webapi::util::read_string;
use sk_runtime::{host_fn, JsRuntime};

const UNDEF: u64 = 0xa;

fn ret_id(id: u32) -> u64 {
    Value::int32(id as i32).0
}

fn arg_id(v: Value) -> u32 {
    v.as_number().unwrap_or(0.0) as u32
}

host_fn!(fn sk_intern(g, args) {
    let name = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    ret_id(tree::with_tree(|t| t.intern(&name)))
});

host_fn!(fn sk_create_element(_g, args) {
    let tag = arg_id(args.arg(0));
    perf::note_dom(perf::DomOp::Create);
    ret_id(tree::with_tree(|t| t.create_element(tag)))
});

host_fn!(fn sk_create_text(_g, _args) {
    perf::note_dom(perf::DomOp::Create);
    ret_id(tree::with_tree(|t| t.create_text()))
});

host_fn!(fn sk_create_anchor(_g, _args) {
    perf::note_dom(perf::DomOp::Create);
    ret_id(tree::with_tree(|t| t.create_anchor()))
});

host_fn!(fn sk_set_text(g, args) {
    let id = arg_id(args.arg(0));
    let text = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Text);
    tree::with_tree(|t| t.set_text(id, text));
    UNDEF
});

host_fn!(fn sk_set_element_text(g, args) {
    let id = arg_id(args.arg(0));
    let text = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Text);
    tree::with_tree(|t| t.set_element_text(id, text));
    UNDEF
});

host_fn!(fn sk_insert(_g, args) {
    let (child, parent, anchor) = (arg_id(args.arg(0)), arg_id(args.arg(1)), arg_id(args.arg(2)));
    perf::note_dom(perf::DomOp::Insert);
    tree::with_tree(|t| t.insert(child, parent, anchor));
    iframe::note_structure();
    UNDEF
});

host_fn!(fn sk_remove(_g, args) {
    let id = arg_id(args.arg(0));
    perf::note_dom(perf::DomOp::Remove);
    tree::with_tree(|t| t.remove(id));
    iframe::note_structure();
    UNDEF
});

host_fn!(fn sk_set_style(g, args) {
    let id = arg_id(args.arg(0));
    let key = arg_id(args.arg(1));
    let value = unsafe { read_string(g, args.arg(2).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Style);
    tree::with_tree(|t| t.set_style(id, key, value));
    UNDEF
});

// The array form of `:style` (`display: ["-webkit-box", "flex"]`) keeps the last value the browser
// accepts, and only the engine can say which one that is here.
host_fn!(fn sk_style_accepts(g, args) {
    let key = arg_id(args.arg(0));
    let value = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let name = tree::with_tree(|t| t.name(key).to_string());
    Value::boolean(sk_css::accepts_declaration(&name, &value)).0
});

host_fn!(fn sk_set_class(g, args) {
    let id = arg_id(args.arg(0));
    let classes = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Class);
    tree::with_tree(|t| t.set_class(id, &classes));
    UNDEF
});

host_fn!(fn sk_set_prop(g, args) {
    let id = arg_id(args.arg(0));
    let key = arg_id(args.arg(1));
    let value = if args.arg(2).is_undefined() || args.arg(2).is_null() {
        None
    } else {
        unsafe { read_string(g, args.arg(2).0) }
    };
    perf::note_dom(perf::DomOp::Prop);
    let (img_src, frame_attr) = tree::with_tree(|t| {
        t.set_prop(id, key, value);
        if t.name(key) == "checked" {
            toggle::attribute_changed(t, id);
        }
        let tag = t.node(id).map(|n| t.name(n.tag));
        let name = t.name(key);
        (name == "src" && tag == Some("img"), tag == Some("iframe") && iframe::is_frame_attr(name))
    });
    if frame_attr {
        iframe::note(id);
    }
    if img_src {
        perf::note_img_src();
        img::note_src(id);
    }
    UNDEF
});

host_fn!(fn sk_add_scope(g, args) {
    let id = arg_id(args.arg(0));
    let scope = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    perf::note_dom(perf::DomOp::Other);
    tree::with_tree(|t| t.add_scope(id, scope));
    UNDEF
});

host_fn!(fn sk_set_stylesheet(g, args) {
    let css = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let extra = args.arg(1).0;
    let head: Vec<String> = if sk_runtime::redjs::webapi::util::is_cell(extra) {
        unsafe {
            use sk_runtime::redjs::webapi::util::{array_len, get_i};
            (0..array_len(g, extra)).filter_map(|i| read_string(g, get_i(g, extra, i))).collect()
        }
    } else {
        Vec::new()
    };
    if tree::set_stylesheet_with_head(css, head) {
        perf::note_full(perf::FullReason::Stylesheet);
        tree::with_tree(|t| t.mark_dirty());
    }
    UNDEF
});

host_fn!(fn sk_set_listener(g, args) {
    let id = arg_id(args.arg(0));
    let event = arg_id(args.arg(1));
    let on = args.arg(2).as_bool().unwrap_or(false);
    let _ = g;
    perf::note_dom(perf::DomOp::Other);
    tree::with_tree(|t| t.set_listener(id, event, on));
    UNDEF
});

host_fn!(fn sk_create_root(_g, _args) {
    ret_id(tree::with_tree(|t| t.create_root()))
});

host_fn!(fn sk_set_dispatcher(g, args) {
    let f = args.arg(0);
    unsafe { sk_runtime::redjs::ffi::redjs_protect(g, f.0) };
    tree::set_dispatcher(f.0);
    UNDEF
});

fn ret_string(s: &str) -> u64 {
    JsRuntime::with(|rt| rt.ctx().string(s).0)
}

// Ids comma-joined, `""` for no match, or `"!<advisory>"` for a selector the engine cannot run.
host_fn!(fn sk_query(g, args) {
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

// Dev-build performance advisories. `__sk_perf_frame()` counts built frames; `__sk_perf_visible(id)`
// answers "<elements>,<in view>" for `id`'s subtree against its scroll container, or "".
sk_runtime::raw_host_fn! { fn sk_perf_frame(_g, _cf) {
    Value::number(render::build_count() as f64).0
} }

host_fn!(fn sk_perf_visible(_g, args) {
    let out = dom::visible_share(arg_id(args.arg(0)))
        .map_or_else(String::new, |(total, seen)| format!("{total},{seen}"));
    ret_string(&out)
});

host_fn!(fn sk_bounds(_g, args) {
    let id = arg_id(args.arg(0));
    perf::note_read(perf::Read::Layout);
    let out = match dom::bounds(id) {
        (Some((x, y, w, h)), _) => match dom::transformed(id) {
            Some((tx, ty, tw, th)) => format!("{x},{y},{w},{h},{tx},{ty},{tw},{th}"),
            None => format!("{x},{y},{w},{h}"),
        },
        (None, true) => "stale".to_string(),
        (None, false) => String::new(),
    };
    ret_string(&out)
});

// `__sk_focus(id, op)`: op 0 `el.focus()` answers "moved:<old id>" (JS fires blur/focusout on the old
// one, then focus/focusin), "already", "pending" (queued for the frame tick) or "refused" (not
// focusable, detached, or hidden by CSS); op 1 `el.blur()` answers "blurred" or "". With no window
// (module-scope code at startup) a focus is queued and a blur is a no-op.
host_fn!(fn sk_focus(_g, args) {
    let id = arg_id(args.arg(0));
    let op = arg_id(args.arg(1));
    if op == 0 && render::style_refuses_focus(id) {
        return ret_string("refused");
    }
    if op == 0 && layout_owed() && render::focus_wants_build(id) {
        sync_layout_by(perf::SyncCaller::Focus);
    }
    let out = sk_runtime::with_commit(|window, cx| {
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

host_fn!(fn sk_input_select(_g, args) {
    Value::boolean(render::select_input_text(arg_id(args.arg(0)))).0
});

// `name=value` lines, empty for a node that is not a loaded `<video>`.
host_fn!(fn sk_media_state(_g, args) {
    let id = arg_id(args.arg(0));
    ret_string(&video::media_state(id).unwrap_or_default())
});

// One scalar media property by code (`video::media_prop`): a number, or `undefined` for a node that
// is not a live media element.
host_fn!(fn sk_media_prop(_g, args) {
    let id = arg_id(args.arg(0));
    let code = arg_id(args.arg(1));
    video::media_prop(id, code).map_or(UNDEF, |v| Value::number(v).0)
});

host_fn!(fn sk_media_call(_g, args) {
    let id = arg_id(args.arg(0));
    let verb = arg_id(args.arg(1));
    let arg = args.arg(2).as_number().unwrap_or(0.0);
    Value::boolean(video::media_call(id, verb, arg)).0
});

// `canPlayType(type)` -> "" | "maybe" | "probably", probed live on this machine — never a static
// table (the owner constraint in sk_video's docs).
host_fn!(fn sk_media_can_play(g, args) {
    let id = arg_id(args.arg(0));
    let mime = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    ret_string(&video::can_play_type(id, &mime))
});

host_fn!(fn sk_media_events(_g, args) {
    let id = arg_id(args.arg(0));
    ret_string(&video::drain_recent(id))
});

host_fn!(fn sk_iframe_post(g, args) {
    let id = arg_id(args.arg(0));
    let wire = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let target = unsafe { read_string(g, args.arg(2).0) }.unwrap_or_default();
    iframe::post(id, &wire, &target);
    UNDEF
});

host_fn!(fn sk_iframe_supported(_g, _args) {
    Value::boolean(iframe::supported()).0
});

host_fn!(fn sk_img_bitmap(g, args) {
    let id = arg_id(args.arg(0));
    let clean = args.arg(1).as_bool().unwrap_or(true);
    let found = sk_runtime::with_app(|cx| img::pixels(id, Some(cx))).unwrap_or_else(|| img::pixels(id, None));
    match found {
        img::Pixels::Ready { width, height, rgba } => {
            let mut data = sk_runtime::redjs::webapi::image::BitmapData::new(width, height, rgba);
            data.origin_clean = clean;
            unsafe { sk_runtime::redjs::webapi::image::create_bitmap(g, data) }
        }
        img::Pixels::NotYet => Value::null().0,
        img::Pixels::Broken(why) => ret_string(&why),
    }
});

host_fn!(fn sk_fullscreen_element(_g, args) {
    Value::boolean(fullscreen::set(arg_id(args.arg(0)))).0
});

pub fn install(rt: &JsRuntime) {
    let fns: &[(&str, u32, *const c_void)] = &[
        ("__sk_intern", 1, sk_intern as *const _),
        ("__sk_create_element", 1, sk_create_element as *const _),
        ("__sk_create_text", 0, sk_create_text as *const _),
        ("__sk_create_anchor", 0, sk_create_anchor as *const _),
        ("__sk_set_text", 2, sk_set_text as *const _),
        ("__sk_set_element_text", 2, sk_set_element_text as *const _),
        ("__sk_insert", 3, sk_insert as *const _),
        ("__sk_remove", 1, sk_remove as *const _),
        ("__sk_set_style", 3, sk_set_style as *const _),
        ("__sk_style_accepts", 2, sk_style_accepts as *const _),
        ("__sk_set_class", 2, sk_set_class as *const _),
        ("__sk_set_prop", 3, sk_set_prop as *const _),
        ("__sk_add_scope", 2, sk_add_scope as *const _),
        ("__sk_set_stylesheet", 1, sk_set_stylesheet as *const _),
        ("__sk_set_listener", 3, sk_set_listener as *const _),
        ("__sk_create_root", 0, sk_create_root as *const _),
        (
            "__sk_fullscreen_element",
            1,
            sk_fullscreen_element as *const _,
        ),
        ("__sk_set_dispatcher", 1, sk_set_dispatcher as *const _),
        ("__sk_query", 3, sk_query as *const _),
        ("__sk_bounds", 1, sk_bounds as *const _),
        ("__sk_perf_frame", 0, sk_perf_frame as *const _),
        ("__sk_perf_visible", 1, sk_perf_visible as *const _),
        ("__sk_focus", 2, sk_focus as *const _),
        ("__sk_input_select", 1, sk_input_select as *const _),
        ("__sk_media_state", 1, sk_media_state as *const _),
        ("__sk_media_prop", 2, sk_media_prop as *const _),
        ("__sk_media_call", 3, sk_media_call as *const _),
        ("__sk_media_can_play", 2, sk_media_can_play as *const _),
        ("__sk_media_events", 1, sk_media_events as *const _),
        ("__sk_img_bitmap", 2, sk_img_bitmap as *const _),
        ("__sk_iframe_post", 3, sk_iframe_post as *const _),
        ("__sk_iframe_supported", 0, sk_iframe_supported as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
    waapi::install(rt);
    style_decl::install(rt);
    css_namespace::install(rt);
    selection::install(rt);
    pointer::install(rt);
    native_dom::install(rt);
    geometry_js::install(rt);
    toggle::install(rt);
    pointer_lock::install(rt);
    observe::install(rt);
    font_api::install(rt);
    effect_texture::install();
    view_transition_js::install(rt);
}

// A browser lays out on read; here layout happens during a frame, so a DOM read before the first
// frame would see 0. The read forces a layout through the host hook ([`sync_layout`]).

thread_local! {
    static SYNC_LAYOUT: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
    static IN_SYNC_LAYOUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static INPUT_DISPATCH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Set inside JS dispatches no gpui listener is under ([`off_input`]).
    static OFF_INPUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// [`render::build_count`] at the last flush that took changes; while equal, nothing has laid it out.
    static FLUSHED_AT_BUILD: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static FLUSHED_BY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True while a gpui listener (or gpui's own draw) has called into JS. The forced layout must not
/// draw then: gpui took the frame's listeners out, and focus listeners run inside `Window::draw`.
pub fn in_input_dispatch() -> bool {
    INPUT_DISPATCH.with(|c| c.get()) > 0
}

pub(crate) struct InputDispatch(bool);

impl InputDispatch {
    pub(crate) fn enter() -> Self {
        if OFF_INPUT.with(|c| c.get()) {
            return Self(false);
        }
        Self::inside_gpui()
    }

    /// The same mark whatever encloses it (listeners gpui runs from inside its draw).
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

/// Runs `f` as JS no gpui listener is under (frame tick, pump, devtools events, `window.defer`).
/// It does not hide an enclosing input dispatch.
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

pub(crate) fn layout_owed() -> bool {
    tree::read_tree(|t| t.is_dirty())
        || FLUSHED_AT_BUILD.with(|c| c.get()) == Some(render::build_count())
}

/// Installs the host's "draw one frame now" callback (`sk-tree` cannot depend on `sk-app`).
pub fn set_sync_layout(f: fn()) {
    SYNC_LAYOUT.with(|c| c.set(Some(f)));
}

/// Folds pending changes and draws one frame so a layout read answers from real rectangles.
/// Re-entrant calls are dropped; a no-op with no hook installed.
pub fn sync_layout() {
    if IN_SYNC_LAYOUT.with(|c| c.get()) {
        return;
    }
    sync_layout_inner();
}

/// [`sync_layout`], recorded in the devtools frame trace.
pub(crate) fn sync_layout_by(caller: perf::SyncCaller) {
    if IN_SYNC_LAYOUT.with(|c| c.get()) {
        return;
    }
    let _profile = sk_runtime::redjs::profiling::span(
        "layout",
        match caller {
            perf::SyncCaller::Bounds => "Forced layout: bounds read",
            perf::SyncCaller::Scroll => "Forced layout: scroll read",
            perf::SyncCaller::Focus => "Forced layout: focus",
        },
    );
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
    let Some(f) = SYNC_LAYOUT.with(|c| c.get()) else {
        return;
    };
    IN_SYNC_LAYOUT.with(|c| c.set(true));
    if flush_changes() {
        FLUSHED_BY_SYNC.with(|c| c.set(true));
    }
    f();
    IN_SYNC_LAYOUT.with(|c| c.set(false));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flush {
    Unchanged,
    Changed,
    /// A forced layout drew after the last change; a drawn frame waits to be presented.
    DrawnBySync,
}

pub fn flush_changes() -> bool {
    flush_changes_kind() != Flush::Unchanged
}

/// Like [`flush_changes`], but tells a wave a forced layout already drew from one that needs a draw.
pub fn flush_changes_kind() -> Flush {
    let dirty = tree::with_tree(|t| t.take_dirty());
    if dirty {
        tree::restyle_all();
        render::gc_node_state();
        render::canvas::gc(|id| tree::read_tree(|t| t.node(id).is_some()));
        tree::animator_gc();
        tree::with_tree(|t| t.release_freed());
        FLUSHED_AT_BUILD.with(|c| c.set(Some(render::build_count())));
    }
    let forced = !IN_SYNC_LAYOUT.with(|c| c.get()) && FLUSHED_BY_SYNC.with(|c| c.replace(false));
    if dirty {
        Flush::Changed
    } else if !forced {
        Flush::Unchanged
    } else if layout_owed() {
        Flush::Changed
    } else {
        Flush::DrawnBySync
    }
}

pub fn animating() -> bool {
    tree::animating()
}

pub fn lotties_animating() -> bool {
    lottie::any_playing()
}

pub fn videos_playing() -> bool {
    video::any_playing()
}

/// Publishes the renderer's D3D11 device so Media Foundation decodes onto gpui's texture pool.
///
/// # Safety
/// `ptr` must be a live `ID3D11Device` COM pointer owned by the caller.
#[cfg(windows)]
pub unsafe fn set_video_device(ptr: *mut c_void, generation: u64) {
    sk_video::mf::set_shared_device(ptr, generation);
}

/// A lazy provider of the renderer's D3D11 device, resolved when the first video backend asks.
#[cfg(windows)]
pub use sk_video::mf::{SharedDeviceProvider, SharedVideoDevice};

/// # Safety
/// The provider's successful lease must retain its live D3D11 device.
#[cfg(windows)]
pub unsafe fn install_video_device_provider(
    provider: SharedDeviceProvider,
) -> Result<(), &'static str> {
    unsafe { sk_video::mf::install_shared_device_provider(provider) }
}

/// The renderer recreated its D3D11 device: every `<sk-surface>` drops its GPU frame and producers
/// hear [`surface::SurfaceEvent::DeviceLost`].
pub fn surface_device_changed() {
    surface::device_changed();
}

/// The renderer recreated its D3D11 device; rebuild every `<video>` on the new one, keeping
/// position and transport state.
pub fn video_device_changed() -> usize {
    video::device_changed()
}

/// Per-frame work that needs a `Window`; call once per frame from the pump, before the draw.
/// Returns `true` when the window must repaint.
pub fn frame_tick(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    // The pump's, not an input listener's: a layout read in a handler may force a frame.
    off_input(|| frame_tick_inner(window, cx))
}

fn frame_tick_inner(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    img::evict_pending(window, cx);
    render::apply_pending_blur(window, cx);
    render::apply_pending_focus(window, cx);
    render::ensure_root_focus(window, cx);
    let settled = tree::advance_animations();
    // `|` not `||`: every part must run each tick.
    let anim = render::dispatch_anim_events(window, cx) | settled;
    let scripted = tree::take_script_dirty();
    img::tick(window, cx);
    iframe::tick(window, cx);
    let lottie_moved = lottie::tick(window, cx);
    let video_moved = video::tick(window, cx);
    let stickers_moved = render::sticker::tick(window, cx);
    let canvas_moved = render::canvas::tick(window);
    let surfaces_moved = surface::node::tick(window, cx);
    let gliding = scroll::tick();
    let scrolled = scroll::dispatch_pending(window, cx) | gliding;
    let fonts_moved = fonts::tick(window, cx);
    let resampled = resample::tick(window, cx);
    let restored = img::residency_tick(window, cx);
    let selecting = selection::tick(window, cx);
    let transition = view_transition::tick(window) | view_transition_js::deliver(window, cx);
    let draw = anim
        | transition
        | scripted
        | lottie_moved
        | video_moved
        | stickers_moved
        | canvas_moved
        | surfaces_moved
        | scrolled
        | fonts_moved
        | resampled
        | restored
        | selecting;
    match live::take() {
        live::Shown::Draw => true,
        live::Shown::Present if !draw => {
            live::present(window);
            false
        }
        _ => draw,
    }
}

/// Per-frame work a SECONDARY window does from its own `render()`: its atlas resources and focus.
/// The JS-facing half stays in [`frame_tick`] (one JS realm, once per frame group).
pub fn window_frame_tick(window: &mut gpui::Window, cx: &mut gpui::App) {
    pointer::arm_hover_tick(window);
    img::evict_pending(window, cx);
    lottie::window_tick(window, cx);
    video::drain_pending_drops_for(window, cx);
    video::drain_pending_uploads(window);
    render::sticker::window_tick(window);
    surface::node::window_tick(window);
    render::apply_pending_blur(window, cx);
    render::apply_pending_focus(window, cx);
    render::ensure_root_focus(window, cx);
}

/// Forgets everything a window held, because it has closed. Its atlas went with it, so tiles it
/// still owed are dropped: another window's `drop_image` could remove keys that window re-inserted.
pub fn forget_window(key: windows::WinKey) {
    windows::forget(key);
    render::sticker::forget_window(key);
    lottie::forget_window(key);
    surface::node::forget_window(key);
    iframe::forget_window(key);
    pointer_lock::window_closed(key);
}

/// A paint-only frame is not presented while the page is hidden (`crate::live`); the draw that
/// shows the window again paints the newest one.
pub fn set_page_hidden(hidden: bool) {
    live::set_hidden(hidden);
}

pub fn set_reduce_motion(on: bool) {
    tree::set_reduce_motion(on);
}

/// Publishes the window environment `@media` is evaluated against (viewport, colour scheme,
/// reduced motion, DPI, display mode). Call every frame; a change dirties the tree and returns `true`.
pub fn set_env(env: sk_css::Env) -> bool {
    let changed = tree::set_env(env);
    if changed {
        perf::note_full(perf::FullReason::Env);
        tree::with_tree(|t| t.mark_dirty());
    }
    changed
}

pub fn set_env_for(key: windows::WinKey, env: sk_css::Env) -> bool {
    tree::set_window_env(key, env)
}

/// The last [`set_env`] value, the source `matchMedia` evaluates against (the primary window's).
pub fn env() -> sk_css::Env {
    tree::env()
}

/// Evaluates one media query against [`env`], as the cascade would. `Err` is the compile failure.
pub fn match_media(query: &str) -> Result<bool, String> {
    sk_css::match_media(query, &tree::env())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(w: f32, h: f32, dark: bool, reduce_motion: bool) -> sk_css::Env {
        sk_css::Env {
            viewport_width: w,
            viewport_height: h,
            dark,
            reduce_motion,
            ..sk_css::Env::default()
        }
    }

    fn opacity_of(id: u32) -> Option<f32> {
        tree::read_tree(|t| {
            t.node(id)
                .and_then(|n| n.computed.as_ref())
                .and_then(|c| c.base.opacity)
        })
    }

    #[test]
    fn env_changes_mark_the_whole_tree_dirty_exactly_once() {
        tree::with_tree(|t| t.take_dirty());
        assert!(set_env(env(800.0, 600.0, false, false)));
        assert!(tree::with_tree(|t| t.take_dirty()));
        assert!(!set_env(env(800.0, 600.0, false, false)));
        assert!(!tree::with_tree(|t| t.take_dirty()));
        assert!(set_env(env(800.0, 600.0, true, false)));
        assert!(tree::with_tree(|t| t.take_dirty()));
        assert!(set_env(env(801.0, 600.0, true, false)));
        assert!(tree::with_tree(|t| t.take_dirty()));
    }

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
        tree::set_engine(sk_css::Engine::compile(
            ".field { opacity: 1 } .field:disabled { opacity: 0.4 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(child), Some(1.0));

        tree::with_tree(|t| {
            let key = t.intern("disabled");
            t.set_prop(child, key, Some("true".into()));
        });
        assert!(flush_changes(), "a prop write must dirty the tree");
        assert_eq!(opacity_of(child), Some(0.4));

        tree::with_tree(|t| {
            let key = t.intern("disabled");
            t.set_prop(child, key, Some("false".into()));
        });
        flush_changes();
        assert_eq!(opacity_of(child), Some(1.0));
    }

    /// Attribute selectors read only the property map; `class` rides `set_class`.
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

    #[test]
    fn an_owed_layout_that_cannot_be_forced_reads_stale() {
        let child = mounted_child();
        assert_eq!(dom::bounds(child), (None, true));
        assert!(flush_changes());
        assert_eq!(dom::bounds(child), (None, true), "flushed is not laid out");
        render::note_build();
        assert_eq!(
            dom::bounds(child),
            (None, false),
            "built and still unpainted: not painted"
        );
    }

    #[test]
    fn a_refused_forced_layout_keeps_the_waves_repaint() {
        fn refuse() {}
        set_sync_layout(refuse);
        let child = mounted_child();
        assert_eq!(dom::bounds(child), (None, true));
        assert!(
            !tree::read_tree(|t| t.is_dirty()),
            "the forced layout flushed"
        );
        assert_eq!(
            flush_changes_kind(),
            Flush::Changed,
            "…and the wave still draws, as nothing did"
        );
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

        let third = mounted_child();
        assert_eq!(dom::bounds(third), (None, false));
        tree::with_tree(|t| t.set_class(third, "moved"));
        assert_eq!(flush_changes_kind(), Flush::Changed);
    }

    /// A dispatch under `off_input` does not count as input, but never hides an enclosing one.
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

    /// `getComputedStyle` is a flush point: reka-ui's `<Presence>` reads style right after an
    /// attribute change, and a stale read unmounted dialogs without animating them.
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
        tree::set_engine(sk_css::Engine::compile(
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

        tree::with_tree(|t| {
            let key = t.intern("data-state");
            t.set_prop(child, key, Some("closed".into()));
        });
        assert_eq!(name_now(), "exit", "the read must see its own mutation");

        assert!(
            flush_changes(),
            "a forced flush still owes the frame a repaint"
        );
    }

    #[test]
    fn an_infinite_animation_dies_with_its_node_when_the_wave_reuses_nothing() {
        tree::set_engine(sk_css::Engine::compile(
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

    #[test]
    fn display_none_stops_the_animations_under_it_and_showing_restarts_them() {
        tree::set_engine(sk_css::Engine::compile(
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

        tree::with_tree(|t| t.set_class(panel, "gone"));
        flush_changes();
        assert!(!animating(), "nothing runs under display: none");
        let cancelled: Vec<_> = tree::take_anim_events()
            .into_iter()
            .filter(|e| e.node == spinner && e.kind == sk_css::AnimEventKind::AnimationCancel)
            .collect();
        assert_eq!(cancelled.len(), 1, "one animationcancel for the spinner");

        let late = tree::with_tree(|t| {
            let div = t.intern("div");
            let late = t.create_element(div);
            t.set_class(late, "spin");
            t.insert(late, panel, 1);
            late
        });
        flush_changes();
        assert!(
            !animating(),
            "a node mounted under display: none does not start ({late})"
        );

        tree::with_tree(|t| t.set_class(panel, ""));
        flush_changes();
        assert!(animating(), "showing the panel restarts what is under it");
        let mut hot = std::collections::HashSet::new();
        tree::moving_paths(std::time::Instant::now(), &mut hot);
        assert!(hot.contains(&spinner) && hot.contains(&late));

        tree::with_tree(|t| t.set_class(spinner, "spin gone"));
        flush_changes();
        hot.clear();
        tree::moving_paths(std::time::Instant::now(), &mut hot);
        assert!(!hot.contains(&spinner) && hot.contains(&late));
    }

    #[test]
    fn only_a_moving_nodes_ancestors_are_hot() {
        tree::set_engine(sk_css::Engine::compile(
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

        tree::with_tree(|t| t.set_class(spinner, ""));
        flush_changes();
        hot.clear();
        tree::moving_paths(std::time::Instant::now(), &mut hot);
        assert!(hot.is_empty(), "nothing moves: {hot:?}");
    }

    /// Custom properties inherit through the real tree, so restyle walks document order, not ids.
    #[test]
    fn custom_properties_inherit_through_the_real_tree() {
        let (mid, leaf) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mid = t.create_element(div);
            let leaf = t.create_element(div);
            // Inserted child-first so id order and document order disagree.
            t.insert(leaf, mid, 0);
            t.insert(mid, root, 0);
            t.set_class(mid, "page");
            t.set_class(leaf, "leaf");
            (mid, leaf)
        });
        tree::set_engine(sk_css::Engine::compile(
            ":root { --v: 0.2 } .page { --v: 0.8 } .leaf { opacity: var(--v) }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(leaf), Some(0.8));

        tree::with_tree(|t| t.set_class(mid, "other"));
        flush_changes();
        assert_eq!(opacity_of(leaf), Some(0.2));
    }

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
        tree::set_engine(sk_css::Engine::compile(".leaf { opacity: var(--v, 1) }"));
        tree::restyle_all();
        assert_eq!(opacity_of(leaf), Some(0.35));
    }

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
        tree::set_engine(sk_css::Engine::compile(
            ".leaf { opacity: 0.1 } .page .leaf { opacity: 0.9 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(leaf), Some(0.1));

        tree::with_tree(|t| t.set_class(mid, "page"));
        flush_changes();
        assert_eq!(opacity_of(leaf), Some(0.9));
    }

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
        tree::set_engine(sk_css::Engine::compile(
            "* { opacity: 0.1 } .stack > * { opacity: 0.5 } .space > * + * { opacity: 0.9 } \
             #list > * { opacity: 0.3 }",
        ));
        tree::restyle_all();
        assert_eq!(
            (opacity_of(first), opacity_of(second)),
            (Some(0.1), Some(0.1))
        );

        tree::with_tree(|t| t.set_class(mid, "stack"));
        flush_changes();
        assert_eq!(
            (opacity_of(first), opacity_of(second)),
            (Some(0.5), Some(0.5))
        );

        tree::with_tree(|t| t.set_class(mid, "space"));
        flush_changes();
        assert_eq!(
            (opacity_of(first), opacity_of(second)),
            (Some(0.1), Some(0.9))
        );

        tree::with_tree(|t| {
            t.set_class(mid, "");
            let key = t.intern("id");
            t.set_prop(mid, key, Some("list".into()));
        });
        flush_changes();
        assert_eq!(
            (opacity_of(first), opacity_of(second)),
            (Some(0.3), Some(0.3))
        );
    }

    /// The class list is a set, as `classList` is: a repeated class must not run its rules twice.
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
        tree::set_engine(sk_css::Engine::compile(
            ".a { --v: 0.25 } .b { --v: 0.75 } .leaf { opacity: var(--v, 1) }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(leaf), Some(0.25));

        tree::with_tree(|t| t.set_class(mid, "b"));
        flush_changes();
        assert_eq!(opacity_of(leaf), Some(0.75));
    }

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
        tree::set_engine(sk_css::Engine::compile(
            ".second { opacity: 0.2 } .marker + .second { opacity: 0.8 }",
        ));
        tree::restyle_all();
        assert_eq!(opacity_of(second), Some(0.2));

        tree::with_tree(|t| t.set_class(first, "marker"));
        flush_changes();
        assert_eq!(opacity_of(second), Some(0.8));
    }

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
        tree::set_engine(sk_css::Engine::compile(
            ".item { opacity: 0.3 } .item:last-child { opacity: 0.7 }",
        ));
        tree::restyle_all();
        assert_eq!(
            opacity_of(only),
            Some(0.7),
            "the only child is the last child"
        );

        tree::with_tree(|t| {
            let div = t.intern("div");
            let second = t.create_element(div);
            t.insert(second, parent, 0);
            t.set_class(second, "item");
        });
        flush_changes();
        assert_eq!(opacity_of(only), Some(0.3));
    }

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
        tree::set_engine(sk_css::Engine::compile(
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
        assert_eq!(
            opacity_of(items[2]),
            Some(0.7),
            "third item is the fourth child now"
        );
        assert_eq!(
            opacity_of(items[3]),
            Some(0.3),
            "and the last one the fifth"
        );

        tree::with_tree(|t| t.remove(first));
        flush_changes();
        assert_eq!(opacity_of(items[2]), Some(0.3));
        assert_eq!(opacity_of(items[3]), Some(0.7));
    }

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
        tree::set_engine(sk_css::Engine::compile(
            ".box { opacity: 0.3 } .box:empty { opacity: 0.7 }",
        ));
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
        assert_eq!(
            opacity_of(boxed),
            Some(0.7),
            "an empty text node is still empty"
        );
        tree::with_tree(|t| t.set_text(text, "hi".into()));
        flush_changes();
        assert_eq!(opacity_of(boxed), Some(0.3), "text with data");
    }

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
        tree::set_engine(sk_css::Engine::compile(
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
        tree::set_engine(sk_css::Engine::compile(
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
        assert_eq!(
            opacity_of(title),
            Some(0.8),
            "the editor arrived next door to the title"
        );
        let _ = panel;
    }

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
        tree::set_engine(sk_css::Engine::compile(".on { opacity: 0.6 }"));
        tree::restyle_all();

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
        tree::set_engine(sk_css::Engine::compile(
            ".box { opacity: 0.1 } @media (min-width: 640px) { .box { opacity: 0.9 } }",
        ));

        set_env(env(500.0, 600.0, false, false));
        flush_changes();
        assert_eq!(opacity_of(child), Some(0.1));

        assert!(set_env(env(700.0, 600.0, false, false)));
        assert!(flush_changes());
        assert_eq!(opacity_of(child), Some(0.9));
    }

    fn inert_of(id: u32) -> Option<bool> {
        tree::read_tree(|t| {
            t.node(id).and_then(|n| n.computed.as_ref()).map(|c| {
                c.extras
                    .pointer_events_none
                    .expect("the cascade resolves this for every element")
            })
        })
    }

    /// `pointer-events` inherits, and an `opacity-0 pointer-events-none` panel parked over the live
    /// one would otherwise take every click aimed at it.
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
        tree::set_engine(sk_css::Engine::compile(
            ".inert { pointer-events: none } .live { pointer-events: auto }",
        ));
        flush_changes();

        assert_eq!(inert_of(panel), Some(true));
        assert_eq!(inert_of(child), Some(true), "inertness inherits");
        assert_eq!(inert_of(opted_in), Some(false), "`auto` opts back in");
        assert_eq!(inert_of(deep), Some(false), "…and that inherits too");
    }

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
        tree::set_engine(sk_css::Engine::compile(
            ".hidden { visibility: hidden } .shown { visibility: visible } .gone { display: none } \
             .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0, 0, 0, 0) }",
        ));
        flush_changes();
        let [under_hidden, shown, under_gone, sr, plain] = ids;
        assert!(
            render::style_refuses_focus(under_hidden),
            "visibility inherits"
        );
        assert!(
            !render::style_refuses_focus(shown),
            "the nearest declaration decides"
        );
        assert!(
            render::style_refuses_focus(under_gone),
            "a display: none subtree is not rendered"
        );
        assert!(
            !render::style_refuses_focus(sr),
            "a screen-reader-only box is visible to CSS"
        );
        assert!(!render::style_refuses_focus(plain));
        assert!(tree::read_tree(
            |t| !render::css_visible(t, under_hidden) && render::css_visible(t, sr)
        ));
    }
}

/// Dispatches an input event into JS and runs its microtask checkpoint so Vue's flush lands in the
/// same frame. Needs `Phase::Commit` published; `false` means a handler called `preventDefault()`.
/// Under a gpui listener the checkpoint is owed and run after gpui's dispatch
/// (`render::settle_owed_later`), where an `onMounted` that measures can lay out.
pub fn dispatch_event(node: u32, event: u32, payload: &EventPayload) -> bool {
    dispatch_event_with(node, event, payload, None)
}

thread_local! {
    /// The pointer position, window CSS px, for the next dispatch only.
    static POINTER_AT: std::cell::Cell<Option<(f64, f64)>> = const { std::cell::Cell::new(None) };
}

/// The next [`dispatch_event`] carries the pointer at (`x`, `y`) as the dispatcher's ninth and tenth
/// arguments (a `wheel` payload has no room for `clientX` / `clientY`).
pub(crate) fn pointer_at_next_dispatch(x: f64, y: f64) {
    POINTER_AT.with(|c| c.set(Some((x, y))));
}

/// [`dispatch_event`] with an object for the event to carry, passed as the dispatcher's eighth
/// argument: a ClipboardEvent's `clipboardData` (`crate::clipboard`).
pub fn dispatch_event_with(
    node: u32,
    event: u32,
    payload: &EventPayload,
    data: Option<Value>,
) -> bool {
    // Taken first, so a position set for a dispatch that goes nowhere never reaches the next one.
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

fn dispatch_event_inner(
    f: u64,
    node: u32,
    event: u32,
    payload: &EventPayload,
    data: Option<Value>,
    at: Option<(f64, f64)>,
) -> bool {
    sk_runtime::bridge::settle_owed_drain();
    // Under a gpui listener, owe the checkpoint before the call (owing holds the engine lock, or JSC
    // drains when the call ends). Nested under a script, the script's own checkpoint runs it.
    let owe = in_input_dispatch();
    if owe {
        sk_runtime::bridge::owe_drain();
    }
    JsRuntime::with(|rt| {
        let key = payload.key.map(|k| rt.ctx().string(k));
        let (x, y) = at.map_or((Value::undefined(), Value::undefined()), |(x, y)| {
            (Value::double(x), Value::double(y))
        });
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
            Err(sk_runtime::redjs::api::Error::Exception(e)) => {
                let err = rt.describe(e);
                eprintln!("[js dispatch] {}\n{}", err.header, err.stack);
                log::record("js", format!("[js dispatch] {}\n{}", err.header, err.stack));
                true
            }
            Err(_) => true,
        };
        if !owe {
            rt.ctx().drain_microtasks();
        }
        proceed
    })
}
