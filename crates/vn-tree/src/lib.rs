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

mod render;
mod tree;

pub use render::build_root;
pub use tree::{with_tree, EV_CLICK};

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

host_fn!(fn vn_add_scope(g, args) {
    let id = arg_id(args.arg(0));
    let scope = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    tree::with_tree(|t| t.add_scope(id, scope));
    UNDEF
});

host_fn!(fn vn_set_stylesheet(g, args) {
    let css = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    tree::set_engine(vn_css::Engine::compile(&css));
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
        ("__vn_add_scope", 2, vn_add_scope as *const _),
        ("__vn_set_stylesheet", 1, vn_set_stylesheet as *const _),
        ("__vn_set_listener", 3, vn_set_listener as *const _),
        ("__vn_create_root", 0, vn_create_root as *const _),
        ("__vn_set_dispatcher", 1, vn_set_dispatcher as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

/// Call after a JS wave: if the tree changed, run the restyle pass (inline
/// parse + cascade) and report `true` so the caller notifies the root view.
pub fn flush_changes() -> bool {
    let dirty = tree::with_tree(|t| t.take_dirty());
    if dirty {
        tree::restyle_all();
    }
    dirty
}

/// Dispatches an input event into JS (through the registered dispatcher) and
/// drains microtasks so Vue's reactive flush lands in the same frame. Must be
/// called with `Phase::Commit` published.
pub fn dispatch_event(node: u32, event: u32) {
    let f = tree::dispatcher();
    if f == 0 {
        return;
    }
    JsRuntime::with(|rt| {
        let args = [Value::int32(node as i32), Value::int32(event as i32)];
        if let Err(vn_runtime::redjs::api::Error::Exception(e)) =
            rt.ctx().call(Value(f), Value::undefined(), &args)
        {
            let err = rt.describe(e);
            eprintln!("[js dispatch] {}\n{}", err.header, err.stack);
        }
        // Vue schedules its patch flush on Promise microtasks: drain now so
        // the DOM ops run inside this very listener's Commit publication.
        rt.ctx().drain_microtasks();
    });
}
