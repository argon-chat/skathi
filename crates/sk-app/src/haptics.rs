//! Pointing-device feedback for the window delivering the application callback.
use gpui::{HapticPattern, HapticTarget};
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::{host_fn, with_commit, JsRuntime};

fn target(value: u64) -> Option<HapticTarget> {
    match dec_f64(value)? {
        1.0 => Some(HapticTarget::Mouse),
        2.0 => Some(HapticTarget::Pen),
        _ => None,
    }
}

host_fn!(fn supported(_g, _args) {
    enc_bool(with_commit(|w, _| w.vn_haptics_supported(HapticTarget::Current)).unwrap_or(false))
});

host_fn!(fn supports_pointer(_g, args) {
    enc_bool(target(args.arg(0).0).and_then(|target| with_commit(|w, _| w.vn_haptics_supported(target))).unwrap_or(false))
});

host_fn!(fn perform(g, args) {
    unsafe {
        let pattern = match read_string(g, args.arg(0).0).as_deref() {
            Some("generic") => HapticPattern::Generic,
            Some("alignment") => HapticPattern::Alignment,
            Some("level-change") => HapticPattern::LevelChange,
            Some("ink-continuous") => HapticPattern::InkContinuous,
            Some("pencil-continuous") => HapticPattern::PencilContinuous,
            _ => return throw_type_error(g, "Unknown haptic pattern"),
        };
        let options = args.arg(1).0;
        let (mut device, mut deferred) = (HapticTarget::Current, false);
        if options != UNDEF && options != NULL {
            if !is_cell(options) { return throw_type_error(g, "Expected haptic options"); }
            let id = get(g, options, "pointerId");
            if id != UNDEF {
                let Some(t) = target(id) else { return FALSE };
                device = t;
            }
            let when = get(g, options, "when");
            if when != UNDEF {
                deferred = match read_string(g, when).as_deref() {
                    Some("now") => false,
                    Some("next-frame") => true,
                    _ => return throw_type_error(g, "Expected now or next-frame"),
                };
            }
        }
        enc_bool(with_commit(|w, _| w.vn_perform_haptic(pattern, device, deferred)).unwrap_or(false))
    }
});

host_fn!(fn stop(_g, args) {
    enc_bool(target(args.arg(0).0).and_then(|target| with_commit(|w, _| w.vn_stop_haptic(target))).unwrap_or(false))
});

pub(crate) fn install(runtime: &JsRuntime) {
    unsafe {
        let g = runtime.raw();
        let class = define_class(
            g,
            &ClassSpec {
                name: "AppHaptics",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[
                    ("supportsPointer", 1, supports_pointer),
                    ("perform", 1, perform),
                    ("stop", 1, stop),
                ],
                getters: &[("supported", supported)],
                accessors: &[],
            },
        );
        let instance = new_object(g, class.proto);
        define(
            g,
            sk_runtime::host::global(g),
            "__sk_appHaptics",
            instance,
            2,
        );
    }
}
