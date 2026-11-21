//! `"memory": "low"` in sk.config.json: the allocators in the process keep no memory in reserve.

use std::ffi::{c_int, c_long};

unsafe extern "C" {
    fn mi_option_set(option: c_int, value: c_long);
}

// Positions in mimalloc.h's `mi_option_e`; a test holds them to the header.
const PURGE_DELAY: c_int = 15;
const PAGE_COMMIT_ON_DEMAND: c_int = 40;

/// Whether the mode is `low`. `SK_MEMORY` decides over the app's own setting.
fn low(from_env: Option<&str>, configured: Option<&str>) -> bool {
    match from_env {
        Some("low") => true,
        Some("default") => false,
        _ => configured == Some("low"),
    }
}

/// Before the first JS context, so the pages its heap starts on follow the mode too.
pub(crate) fn apply() {
    let from_env = std::env::var("SK_MEMORY").ok();
    let configured = crate::boot_mode::memory();
    if !low(from_env.as_deref(), configured.as_deref()) {
        return;
    }
    // GPU drivers ship a mimalloc of their own (NVIDIA's does) that reads its options from the
    // environment when the driver loads.
    for (name, value) in [
        ("MIMALLOC_PAGE_COMMIT_ON_DEMAND", "1"),
        ("MIMALLOC_PURGE_DELAY", "0"),
    ] {
        if std::env::var_os(name).is_none() {
            std::env::set_var(name, value);
        }
    }
    // SAFETY: mimalloc's options are atomics, set here before any other thread allocates from it.
    unsafe {
        // A page is committed as it fills, not whole when it is taken.
        mi_option_set(PAGE_COMMIT_ON_DEMAND, 1);
        // Freed memory goes back to the OS at once, not after the delay that expects it to be reused.
        mi_option_set(PURGE_DELAY, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_decides_over_the_app() {
        assert!(low(Some("low"), None));
        assert!(!low(Some("default"), Some("low")));
        assert!(low(None, Some("low")));
        assert!(!low(None, None));
        assert!(low(Some("nonsense"), Some("low")));
    }

    #[test]
    fn the_option_numbers_are_the_headers() {
        let header = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../vendor/redjs/redjs/cpp/mimalloc/include/mimalloc.h"
        );
        let text = std::fs::read_to_string(header).expect("vendor/redjs/setup fetches mimalloc");
        let body = text
            .split("typedef enum mi_option_e {")
            .nth(1)
            .expect("the option enum");
        let names: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("mi_option_"))
            .map(|line| line.split(',').next().unwrap())
            .collect();
        assert_eq!(names[PURGE_DELAY as usize], "mi_option_purge_delay");
        assert_eq!(
            names[PAGE_COMMIT_ON_DEMAND as usize],
            "mi_option_page_commit_on_demand"
        );
    }
}
