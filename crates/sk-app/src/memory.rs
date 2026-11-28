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
pub(crate) fn apply(standalone: bool) {
    let from_env = std::env::var("SK_MEMORY").ok();
    let configured = crate::boot_mode::memory();
    if !low(from_env.as_deref(), configured.as_deref()) {
        return;
    }
    configure_driver(standalone);
    // SAFETY: mimalloc's options are atomic and the option IDs are checked against its header.
    unsafe {
        // A page is committed as it fills, not whole when it is taken.
        mi_option_set(PAGE_COMMIT_ON_DEMAND, 1);
        // Freed memory goes back to the OS at once, not after the delay that expects it to be reused.
        mi_option_set(PURGE_DELAY, 0);
    }
}

/// A GPU driver can load a separate mimalloc that takes its options from the environment.
fn configure_driver(_standalone: bool) {
    let defaults = [
        ("MIMALLOC_PAGE_COMMIT_ON_DEMAND", "1"),
        ("MIMALLOC_PURGE_DELAY", "0"),
    ]
    .into_iter()
    .filter(|(name, _)| std::env::var_os(name).is_none());
    #[cfg(windows)]
    for (name, value) in defaults {
        // SAFETY: std guarantees process environment writes are thread-safe on Windows.
        unsafe { std::env::set_var(name, value) };
    }
    #[cfg(unix)]
    if _standalone {
        use std::os::unix::process::CommandExt;
        let mut defaults = defaults.peekable();
        if defaults.peek().is_none() {
            return;
        }
        // Only the SDK executable may replace itself. exec keeps its PID and sets both defaults
        // before foreign threads can read them, including threads started by native initializers.
        let error = match std::env::current_exe() {
            Ok(exe) => {
                let mut args = std::env::args_os();
                let mut command = std::process::Command::new(exe);
                if let Some(name) = args.next() {
                    command.arg0(name);
                }
                command.args(args).envs(defaults).exec()
            }
            Err(error) => error,
        };
        eprintln!("[memory] could not apply GPU driver allocator defaults: {error}");
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
