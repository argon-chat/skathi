//! Closing the app: the windows go at once, the teardown runs behind them,
//! under a deadline (m26).
//!
//! A close the app accepts hides every window of the process before anything
//! else, as a browser and Electron do. That is `appWindow.close()` /
//! `window.close()` (the titlebar X a frameless app draws), or a WM_CLOSE
//! (the system menu, the taskbar, Alt+F4) that `beforeunload` did not veto.
//! What follows runs with nothing on screen: gpui's quit, the `on_app_quit`
//! hooks (IndexedDB's checkpoint, the cookie jar, the voice rooms' leave) and
//! the end of the process.
//!
//! ```text
//!   close accepted ── begin: hide every window, launch::closing(), arm the deadline
//!        │
//!   gpui quit ── on_app_quit hooks, each timed by `step`
//!        │
//!   end of main ── finish: one `[shutdown]` line, then the process ends
//!
//!   DEADLINE after begin, still running ── one `[shutdown]` line, the process ends
//! ```
//!
//! **The deadline.** [`DEADLINE`] after the close, the process ends whatever
//! is still running, and says what it was doing on stderr. Each quit hook has
//! its own, shorter budget, so the deadline only fires when something has no
//! budget or ignores it. A second launch of a single-instance app that comes
//! in meanwhile is told the instance is closing, waits for this process to
//! end, and starts afresh (crates/vn-desktop/src/launch).
//!
//! **Embedded** (src/embed.rs, docs/EMBEDDING.md) the process is not ours:
//! [`finish`] and the deadline tell the embedder the app has closed, and the
//! vue-native thread sleeps instead of ending anything.
//!
//! **The end.** Once the quit hooks have run, [`finish`] ends the process with
//! `TerminateProcess` instead of returning from `main`. Returning runs every
//! thread-local destructor (the JS realm's, the voice engines') and every
//! DLL's detach, and `ExitProcess` kills the deadline's thread before those
//! begin, so nothing would bound them. None of them writes anything a user
//! would miss: everything that persists is flushed by a quit hook. On an idle
//! Argon host returning cost ~12 ms (m26); the point is the bound, not the
//! time. Browsers end a renderer the same way.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long the teardown may take after a close before the process is ended
/// anyway. Above the quit hooks' own budgets run back to back on an app that
/// has both (IndexedDB 2 s, the voice rooms 2 s, when both are at their
/// worst), and below a second launch's patience with a closing instance
/// (vn-desktop's `CLOSING_PATIENCE`).
pub const DEADLINE: Duration = Duration::from_secs(4);

/// When the close was accepted, and what it was.
static BEGAN: OnceLock<(Instant, &'static str)> = OnceLock::new();
/// The timed teardown steps, in order: when each began, and how long it took
/// (`None` while it runs, which is what the deadline's line names).
static STEPS: Mutex<Vec<(&'static str, Instant, Option<Duration>)>> = Mutex::new(Vec::new());
/// How many windows `begin` hid.
static HIDDEN: Mutex<usize> = Mutex::new(0);
/// Set by whichever of `finish` and the deadline ends the process first.
static ENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn is_closing() -> bool {
    BEGAN.get().is_some()
}

/// A close was accepted (`why` names it): hide every window, tell the
/// single-instance server, arm the deadline. Idempotent; main thread.
pub fn begin(why: &'static str) {
    if BEGAN.set((Instant::now(), why)).is_err() {
        return;
    }
    let hidden = hide_windows();
    if let Ok(mut h) = HIDDEN.lock() {
        *h = hidden;
    }
    vn_desktop::launch::closing();
    vn_tree::log::record("shutdown", format!("[shutdown] {why}: hid {hidden} window(s); tearing down"));
    let _ = std::thread::Builder::new().name("vn-shutdown-deadline".into()).spawn(|| {
        std::thread::sleep(DEADLINE);
        if ENDING.swap(true, Ordering::SeqCst) {
            return;
        }
        // Embedded, the process is the embedder's to end: it hears that the
        // app is gone, unclean, and the teardown thread is left where it is.
        if crate::embed::active() {
            let line = timeline();
            eprintln!("[shutdown] the teardown passed its {} s deadline. {line}", DEADLINE.as_secs());
            crate::embed::finished(false, line);
            return;
        }
        eprintln!(
            "[shutdown] the teardown passed its {} s deadline; ending the process. {}",
            DEADLINE.as_secs(),
            timeline()
        );
        terminate();
    });
}

/// Runs one teardown step and records how long it took, for the line
/// [`finish`] and the deadline print.
pub fn step<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let index = STEPS.lock().map(|mut s| {
        s.push((name, t, None));
        s.len() - 1
    });
    let out = f();
    if let (Ok(i), Ok(mut s)) = (index, STEPS.lock()) {
        s[i].2 = Some(t.elapsed());
    }
    out
}

/// The end of `main`, after gpui's quit and its hooks: one `[shutdown]` line,
/// then the process ends (module doc, "The end").
pub fn finish() -> ! {
    // A quit that did not come through a close (the session ending, an
    // inspector whose app is gone) is timed from here.
    begin("quit");
    if ENDING.swap(true, Ordering::SeqCst) {
        // The deadline is ending the process right now.
        loop {
            std::thread::park();
        }
    }
    eprintln!("[shutdown] closed. {}", timeline());
    // Embedded: the embedder's process goes on. This thread may not end —
    // its thread-locals are the JS realm, whose teardown this JSC build
    // fast-fails (vn-runtime's invariants) — so it reports and sleeps.
    if crate::embed::active() {
        crate::embed::finished(true, timeline());
        loop {
            std::thread::park();
        }
    }
    terminate()
}

/// `VN_SHUTDOWN_DELAY_MS=<n>`: a quit hook that takes `n` ms — the test seam
/// for what a slow teardown looks like from outside: the window gone at
/// once, the deadline, a second launch that arrives while the app is closing
/// (packages/vn-test/specs/close.spec.ts).
pub fn install_test_delay(cx: &mut gpui::App) {
    let Some(ms) = std::env::var("VN_SHUTDOWN_DELAY_MS").ok().and_then(|v| v.trim().parse::<u64>().ok()) else {
        return;
    };
    std::mem::forget(cx.on_app_quit(move |_| {
        step("VN_SHUTDOWN_DELAY_MS", || std::thread::sleep(Duration::from_millis(ms)));
        std::future::ready(())
    }));
}

/// `why: hid N window(s); quit hooks: a 1 ms, b 30 ms, c still running after
/// 3950 ms; ended at +D ms`.
fn timeline() -> String {
    let Some((t0, why)) = BEGAN.get() else { return String::new() };
    let hidden = HIDDEN.lock().map(|h| *h).unwrap_or(0);
    let steps = STEPS
        .lock()
        .map(|s| {
            s.iter()
                .map(|(n, began, took)| match took {
                    Some(d) => format!("{n} {} ms", d.as_millis()),
                    None => format!("{n} still running after {} ms", began.elapsed().as_millis()),
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    format!(
        "{why}: hid {hidden} window(s); quit hooks: {}; ended at +{} ms",
        if steps.is_empty() { "none ran" } else { &steps },
        t0.elapsed().as_millis()
    )
}

#[cfg(target_os = "windows")]
fn hide_windows() -> usize {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, IsWindowVisible, ShowWindow, SW_HIDE};

    // Every top-level window of the UI thread, which is every gpui window of
    // the process: the primary and each pop-out. Synchronous, so the windows
    // are gone from the screen before this returns. gpui hears the change
    // (`WM_SHOWWINDOW`) and reports it later, from its executor, so this is
    // safe inside a window update.
    unsafe extern "system" fn hide(hwnd: HWND, count: LPARAM) -> BOOL {
        unsafe {
            if IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_HIDE);
                *(count.0 as *mut usize) += 1;
            }
        }
        true.into()
    }
    let mut count = 0usize;
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(hide), LPARAM(&mut count as *mut usize as isize));
    }
    count
}

#[cfg(not(target_os = "windows"))]
fn hide_windows() -> usize {
    0
}

fn terminate() -> ! {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        let _ = TerminateProcess(GetCurrentProcess(), 0);
    }
    std::process::exit(0)
}
