//! Elements whose engine this build left out: they lay out as an empty box, and creating one
//! warns once naming the Cargo feature that brings it back.

use std::ffi::c_void;

const MISSING: &[(&str, &str, &str)] = &[
    #[cfg(not(feature = "video"))]
    ("video", "video", "plays nothing"),
    #[cfg(not(feature = "lottie"))]
    ("lottie", "lottie", "draws nothing"),
    #[cfg(not(feature = "lottie"))]
    ("sk-sticker", "lottie", "draws nothing"),
];

pub(crate) fn is_absent(tag: &str) -> bool {
    MISSING.iter().any(|m| m.0 == tag)
}

pub(crate) fn advise(g: *mut c_void, tag: u32) {
    let Some(&(name, feature, effect)) =
        crate::tree::with_tree(|t| MISSING.iter().find(|m| m.0 == t.name(tag)))
    else {
        return;
    };
    let msg = format!(
        "<{name}> {effect}: this app's host was built without Skathi's `{feature}` feature. \
         Enable it in the `skathi` dependency's features to use <{name}>."
    );
    // SAFETY: `g` is the live context of the host function that called this.
    unsafe { sk_runtime::host::warn_compat(g, &format!("absent:{name}"), &msg) };
}
