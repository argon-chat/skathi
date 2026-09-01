//! Where THIS process keeps the files that identify it: the dist slot.
//!
//! One place, because three subsystems have to agree on it and two of them now
//! exist in builds the third does not:
//!
//! * the dev transport reads `manifest.json` / `changes.json` / the module
//!   scripts out of it (`crate::dev`, dev builds only);
//! * the devtools listener publishes its handshake `devtools.json` into it, and
//!   discovery scans its siblings (`crate::devtools`, `cfg(vn_devtools)`);
//! * the embedded inspector is pointed at a slot by `--target`.
//!
//! `--features prod,devtools` is the build where those two sets stop
//! overlapping: there is no `dist/`, no manifest and no HMR, but there IS a
//! devtools port that has to be discoverable. So the slot degrades to the
//! executable's own directory, which is the one place a shipped app certainly
//! owns and the one a second instance of it certainly shares.

use std::path::PathBuf;

/// The compile-time default: the dev builder's output directory.
const DEV_DIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/vue-native/dist");

/// This process's slot directory.
///
/// `VN_DIST` wins everywhere — it is what points one exe at another app's build
/// output, and it is how every parallel host in this repo (the e2e slot pool,
/// the two-window voice demo, the inspector itself) stays independently
/// addressable.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VN_DIST").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    #[cfg(not(feature = "prod"))]
    {
        PathBuf::from(DEV_DIST)
    }
    #[cfg(feature = "prod")]
    {
        // The exe's directory. `current_exe` can fail on paper; a shipped app
        // that cannot name its own binary has bigger problems than devtools
        // discovery, so the fallback is the working directory rather than a
        // panic.
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// A file inside this process's slot.
pub fn path(rel: &str) -> PathBuf {
    dir().join(rel)
}

#[cfg(all(test, not(feature = "prod")))]
mod tests {
    #[test]
    fn the_dev_default_is_the_builders_output_directory() {
        // Not asserting the absolute path (it embeds the checkout), only that
        // the two halves agree about the name — `dev.rs` used to own this
        // constant and `devtools` reached into it.
        assert!(super::DEV_DIST.ends_with("packages/vue-native/dist"));
    }
}
