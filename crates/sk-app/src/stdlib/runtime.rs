//! `skathi:runtime`: what the host knows about itself and about how the app reached it.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use sk_runtime::host_fn;
use sk_runtime::redjs::webapi::util::*;

use crate::payload::{self, Source};

static APP_NATIVE: AtomicBool = AtomicBool::new(false);

/// Whether the app's code is running from its native image; a reload runs the bundle instead.
#[cfg_attr(not(feature = "native-aot"), allow(dead_code))]
pub(crate) fn set_app_native(native: bool) {
    APP_NATIVE.store(native, Ordering::Relaxed);
}

const FEATURES: &[(&str, bool)] = &[
    ("audio", cfg!(feature = "audio")),
    ("spatial-audio", cfg!(feature = "spatial-audio")),
    ("storages", cfg!(feature = "storages")),
    ("clipboard", cfg!(feature = "clipboard")),
    ("indexeddb", cfg!(feature = "indexeddb")),
    ("webgpu", cfg!(feature = "webgpu")),
    ("lottie", cfg!(feature = "lottie")),
    ("video", cfg!(feature = "video")),
    ("hotkeys", cfg!(feature = "hotkeys")),
    ("webview", cfg!(feature = "webview")),
    ("battery", cfg!(feature = "battery")),
    ("devtools", cfg!(sk_devtools)),
    ("native-aot", cfg!(feature = "native-aot")),
    ("no-jit", cfg!(feature = "no-jit")),
    ("no-wasm", cfg!(feature = "no-wasm")),
];

/// The app needs no file beside the executable: the payload is inside it, with no packs, ICU data
/// or native image to load from disk. A macOS bundle ships and is signed as one thing.
fn single_file() -> bool {
    let Some(loaded) = payload::get() else {
        return false;
    };
    if matches!(loaded.source, Source::Bundled(_)) {
        return true;
    }
    if loaded.payload.is_file() || !loaded.meta.packs.is_empty() {
        return false;
    }
    #[cfg(feature = "native-aot")]
    if crate::native_aot::app_image_beside() {
        return false;
    }
    let wants_icu_pack =
        redjs::icu::LINKED == redjs::icu::Linked::Root && loaded.meta.intl != "invariant";
    !wants_icu_pack || loaded.payload.icu_data().is_some()
}

host_fn!(fn version(g, _args) {
    unsafe { js_string(g, env!("SK_PKG_VERSION")) }
});

host_fn!(fn is_aot(_g, _args) {
    enc_bool(APP_NATIVE.load(Ordering::Relaxed))
});

host_fn!(fn is_single_file(_g, _args) {
    enc_bool(single_file())
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let class = define_class(
        g,
        &ClassSpec {
            name: "Runtime",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[],
            getters: &[("version", version)],
            accessors: &[],
        },
    );
    define(g, class.ctor, "isAot", make_fn(g, "isAot", 0, is_aot), 2);
    define(
        g,
        class.ctor,
        "isSingleFile",
        make_fn(g, "isSingleFile", 0, is_single_file),
        2,
    );
    let names = new_array(g);
    for (i, (name, _)) in FEATURES.iter().filter(|(_, on)| *on).enumerate() {
        set_i(g, names, i as u32, js_string(g, name));
    }
    let global = sk_runtime::host::global(g);
    let construct = get(g, get(g, global, "Reflect"), "construct");
    let arguments = new_array(g);
    set_i(g, arguments, 0, names);
    let features = call(g, construct, UNDEF, &[get(g, global, "Set"), arguments]).unwrap_or(UNDEF);
    let instance = new_object(g, class.proto);
    define(g, instance, "features", features, 1 | 4);
    define(g, std, "Runtime", class.ctor, 0);
    define(g, std, "runtime", instance, 0);
}
