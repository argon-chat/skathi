//! Pointing-device feedback for the window delivering the application callback.
use gpui::{HapticPattern, HapticTarget};
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::{host_fn, with_commit, JsRuntime};
use std::ffi::c_void;

unsafe fn pattern(g: *mut c_void, value: u64) -> Option<HapticPattern> {
    let name = unsafe { read_string(g, value) }?;
    HapticPattern::NAMED
        .iter()
        .find(|&&(key, _)| key == name)
        .map(|&(_, p)| p)
}

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

host_fn!(fn supported_patterns(g, args) {
    let id = args.arg(0).0;
    let device = if id == UNDEF { Some(HapticTarget::Current) } else { target(id) };
    let available = device.and_then(|d| with_commit(|w, _| w.vn_haptic_patterns(d))).unwrap_or_default();
    unsafe {
        let result = new_array(g);
        for (i, (name, _)) in HapticPattern::NAMED.iter().filter(|(_, p)| available.contains(p)).enumerate() {
            set_i(g, result, i as u32, js_string(g, name));
        }
        result
    }
});

host_fn!(fn perform(g, args) {
    unsafe {
        let Some(requested) = pattern(g, args.arg(0).0) else { return throw_type_error(g, "Unknown haptic pattern") };
        let options = args.arg(1).0;
        let (mut device, mut deferred) = (HapticTarget::Current, false);
        let mut fallback = UNDEF;
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
            fallback = get(g, options, "fallback");
        }
        let accepted = match fallback {
            UNDEF => with_commit(|w, _| w.vn_perform_haptic(requested, device, deferred)),
            FALSE => with_commit(|w, _| w.vn_perform_first_haptic(&[requested], device, deferred)),
            value => {
                let Some(alternate) = pattern(g, value) else { return throw_type_error(g, "Expected a haptic fallback pattern or false") };
                with_commit(|w, _| w.vn_perform_first_haptic(&[requested, alternate], device, deferred))
            }
        };
        enc_bool(accepted.unwrap_or(false))
    }
});

host_fn!(fn stop(_g, args) {
    enc_bool(target(args.arg(0).0).and_then(|target| with_commit(|w, _| w.vn_stop_haptic(target))).unwrap_or(false))
});

host_fn!(fn trigger(g, args) {
    unsafe {
        let element = args.arg(0).0;
        let Some(name) = read_string(g, args.arg(1).0) else { return throw_type_error(g,"Expected a haptic trigger") };
        let Some(gesture) = sk_css::haptics::Trigger::from_name(&name) else { return throw_type_error(g,"Unknown haptic trigger") };
        if !is_cell(element) { return throw_type_error(g,"Expected an element"); }
        let Some(id) = dec_f64(get(g,element,"__skId")).filter(|n| *n > 0.) else { return FALSE };
        let input = args.arg(2).0;
        let source = if input == UNDEF { HapticTarget::Current } else if let Some(t) = target(input) { t } else { return FALSE };
        enc_bool(with_commit(|w,cx| sk_tree::haptics::trigger(id as u32,gesture,source,w,cx)).unwrap_or(false))
    }
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
                    ("getSupportedPatterns", 0, supported_patterns),
                    ("trigger", 2, trigger),
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
