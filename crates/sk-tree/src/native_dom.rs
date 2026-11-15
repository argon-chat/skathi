//! Shared pieces of the native DOM members: the node a receiver mirrors, the mirror of a node id
//! (asked of the runtime's registry), and members on the engine's prototypes.

use std::cell::Cell;
use std::ffi::c_void;

use sk_runtime::host::global;
use sk_runtime::host_fn;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::JsRuntime;

thread_local! {
    /// `(context, nodeById)`, as the runtime registered it.
    static LOOKUP: Cell<(usize, u64)> = const { Cell::new((0, UNDEF)) };
}

/// The live node `this` mirrors: `None` for anything else, and for a freed mirror.
pub(crate) unsafe fn node_id(g: *mut c_void, this: u64) -> Option<u32> {
    if !is_cell(this) {
        return None;
    }
    dec_f64(get(g, this, "__skId"))
        .filter(|&id| id > 0.0)
        .map(|id| id as u32)
}

/// The mirror of node `id`, or `null`.
pub(crate) unsafe fn node_by_id(g: *mut c_void, id: u32) -> u64 {
    let lookup = match LOOKUP.get() {
        (ctx, f) if ctx == g as usize && id != 0 => f,
        _ => return NULL,
    };
    match call(g, lookup, UNDEF, &[enc_f64(f64::from(id))]) {
        Ok(v) if is_cell(v) => v,
        _ => NULL,
    }
}

host_fn!(fn set_node_lookup(g, args) {
    let f = args.arg(0).0;
    let (ctx, old) = LOOKUP.replace((g as usize, if is_cell(f) { f } else { UNDEF }));
    unsafe {
        if is_cell(f) {
            redjs_protect(g, f);
        }
        if is_cell(old) && ctx == g as usize {
            redjs_unprotect(g, old);
        }
    }
    UNDEF
});

/// `globalThis[name].prototype`.
pub(crate) unsafe fn proto(g: *mut c_void, name: &str) -> u64 {
    let ctor = get(g, global(g), name);
    if is_cell(ctor) {
        get(g, ctor, "prototype")
    } else {
        UNDEF
    }
}

pub(crate) unsafe fn method(g: *mut c_void, obj: u64, name: &str, arity: u32, f: HostFn) {
    define(g, obj, name, make_fn(g, name, arity, f), 2);
}

/// A configurable accessor, read-only without `set`.
pub(crate) unsafe fn accessor(
    g: *mut c_void,
    obj: u64,
    name: &str,
    get_fn: HostFn,
    set_fn: Option<HostFn>,
) {
    let a = atom(g, name);
    let setter = set_fn.map_or(UNDEF, |f| make_fn(g, &format!("set {name}"), 1, f));
    glue::rjs_define_accessor(
        g,
        obj,
        a,
        make_fn(g, &format!("get {name}"), 0, get_fn),
        setter,
        2,
    );
    glue::rjs_atom_free(g, a);
}

pub(crate) fn install(rt: &JsRuntime) {
    rt.register("__sk_set_node_lookup", 1, set_node_lookup as *const c_void);
}
