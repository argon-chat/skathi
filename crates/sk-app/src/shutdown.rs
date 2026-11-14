//! Closing the app: the windows go at once, the teardown runs behind them, under a deadline.
//!
//! A close the app accepts (`appWindow.close()` / `window.close()`, or a WM_CLOSE that
//! `beforeunload` did not veto) hides every window of the process first, as a browser does. What
//! follows runs with nothing on screen: gpui's quit, the `on_app_quit` hooks (IndexedDB's
//! checkpoint, the cookie jar, the voice rooms' leave) and the end of the process.
//!
//! **The deadline.** [`DEADLINE`] after the close the process ends whatever is still running, and
//! says on stderr what it was doing. Each quit hook has its own shorter budget, so it fires only
//! when something has none or ignores it. A second launch of a single-instance app meanwhile is told
//! the instance is closing, waits for this process to end, and starts afresh.
//!
//! **Embedded** (src/embed.rs) the process is not ours: [`finish`] and the deadline tell the
//! embedder the app has closed, and the Skathi thread sleeps instead of ending anything.
//!
//! **The end.** After the quit hooks, [`finish`] ends the process with `TerminateProcess` instead of
//! returning from `main`: returning runs every thread-local destructor and DLL detach, and
//! `ExitProcess` kills the deadline's thread before those begin, so nothing would bound them. None
//! writes anything a user would miss, since quit hooks flush everything that persists.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long the teardown may take after a close before the process is ended anyway: above the quit
/// hooks' budgets run back to back (IndexedDB 2 s, voice rooms 2 s), below a second launch's
/// patience with a closing instance (sk-desktop's `CLOSING_PATIENCE`).
pub const DEADLINE: Duration = Duration::from_secs(4);

static BEGAN: OnceLock<(Instant, &'static str)> = OnceLock::new();
static STEPS: Mutex<Vec<(&'static str, Instant, Option<Duration>)>> = Mutex::new(Vec::new());
static HIDDEN: Mutex<usize> = Mutex::new(0);
static ENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn is_closing() -> bool {
    BEGAN.get().is_some()
}

pub fn begin(why: &'static str) {
    if BEGAN.set((Instant::now(), why)).is_err() {
        return;
    }
    let hidden = hide_windows();
    if let Ok(mut h) = HIDDEN.lock() {
        *h = hidden;
    }
    sk_desktop::launch::closing();
    sk_tree::log::record("shutdown", format!("[shutdown] {why}: hid {hidden} window(s); tearing down"));
    let _ = std::thread::Builder::new().name("sk-shutdown-deadline".into()).spawn(|| {
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

pub fn finish() -> ! {
    begin("quit");
    if ENDING.swap(true, Ordering::SeqCst) {
        loop {
            std::thread::park();
        }
    }
    eprintln!("[shutdown] closed. {}", timeline());
    // Embedded: the embedder's process goes on. This thread may not end —
    // its thread-locals are the JS realm, whose teardown this JSC build
    // fast-fails (sk-runtime's invariants) — so it reports and sleeps.
    if crate::embed::active() {
        crate::embed::finished(true, timeline());
        loop {
            std::thread::park();
        }
    }
    terminate()
}

/// `SK_SHUTDOWN_DELAY_MS=<n>`: a quit hook that takes `n` ms, the test seam for a slow teardown
/// (packages/test/specs/close.spec.ts).
pub fn install_test_delay(cx: &mut gpui::App) {
    let Some(ms) = std::env::var("SK_SHUTDOWN_DELAY_MS").ok().and_then(|v| v.trim().parse::<u64>().ok()) else {
        return;
    };
    std::mem::forget(cx.on_app_quit(move |_| {
        step("SK_SHUTDOWN_DELAY_MS", || std::thread::sleep(Duration::from_millis(ms)));
        std::future::ready(())
    }));
}

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

    // Every top-level window of the UI thread, which is every gpui window (primary and pop-outs).
    // Synchronous; gpui hears `WM_SHOWWINDOW` later from its executor, so this is safe inside a
    // window update.
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
