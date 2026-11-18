//! `AudioContext.prototype.spatialAudio` over sk-audio: whether the context's `"spatial"` panners
//! render as Windows spatial audio objects, through which format, and why not.

use std::ffi::c_void;

use sk_runtime::host::{getter, global};
use sk_runtime::redjs::webapi::audio::output_status;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::JsRuntime;

pub fn install(rt: &JsRuntime) {
    sk_audio::install();
    let g = rt.raw();
    // SAFETY: the runtime's own context, on its thread.
    unsafe {
        let ctor = get(g, global(g), "AudioContext");
        if is_cell(ctor) {
            let proto = get(g, ctor, "prototype");
            if is_cell(proto) {
                getter(g, proto, "spatialAudio", spatial_audio);
            }
        }
    }
}

sk_runtime::host_fn!(fn spatial_audio(g, args) {
    unsafe { status(g, args.this().0) }
});

unsafe fn status(g: *mut c_void, this: u64) -> u64 {
    let Some(s) = output_status(g, this) else {
        return throw_type_error(g, "Illegal invocation");
    };
    let o = new_object(g, UNDEF);
    let active = s.objects.as_ref();
    define(g, o, "active", enc_bool(active.is_some()), 0);
    define(g, o, "requested", enc_bool(s.objects_requested), 0);
    let format = active.map_or(NULL, |a| js_string(g, &a.label));
    define(g, o, "format", format, 0);
    define(
        g,
        o,
        "maxObjects",
        enc_i32(active.map_or(0, |a| a.max_objects) as i32),
        0,
    );
    define(
        g,
        o,
        "objectsInUse",
        enc_i32(active.map_or(0, |a| a.in_use) as i32),
        0,
    );
    let reason = match (&s.objects, &s.refused) {
        (None, Some(r)) if s.objects_requested => js_string(g, r),
        _ => NULL,
    };
    define(g, o, "reason", reason, 0);
    o
}
