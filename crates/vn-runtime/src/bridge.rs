//! The JS↔gpui call boundary: raw `&mut App` / `&mut Window` pointers are
//! published for a bounded scope with an RAII guard, tagged with a [`Phase`],
//! and host functions may only reach them through [`with_commit`] /
//! [`with_app`]. Outside a published scope those return `None` and the caller
//! must throw a JS error rather than touch gpui.
//!
//! This is the `ElementArenaScope` pattern from gpui itself: teardown lives in
//! `Drop`, so a panic cannot leave a dangling pointer published.

use std::any::Any;
use std::cell::Cell;
use std::ffi::c_void;

use gpui::{App, Window};

/// Where the current thread stands relative to gpui's frame lifecycle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Nothing published. Host fns needing gpui must throw.
    None,
    /// `&mut App` published, no window (app-level tasks, startup eval).
    App,
    /// `&mut App` + `&mut Window` published, outside any draw. The only phase
    /// in which the retained tree may be mutated and gpui called freely.
    Commit,
    /// Inside `Window::draw`: everything is frozen, nothing is published.
    Draw,
}

thread_local! {
    static APP: Cell<*mut App> = const { Cell::new(std::ptr::null_mut()) };
    static WINDOW: Cell<*mut Window> = const { Cell::new(std::ptr::null_mut()) };
    static PHASE: Cell<Phase> = const { Cell::new(Phase::None) };
    static POISONED: Cell<bool> = const { Cell::new(false) };
    static LAST_PANIC: Cell<Option<(String, String)>> = const { Cell::new(None) };
}

/// True once a host-fn panic has been caught. gpui state must be assumed
/// corrupt (leases dropped, update depth unbalanced): stop scheduling JS.
pub fn poisoned() -> bool {
    POISONED.with(|c| c.get())
}

pub fn poison() {
    POISONED.with(|c| c.set(true));
}

struct Guard(*mut App, *mut Window, Phase);

impl Drop for Guard {
    fn drop(&mut self) {
        APP.with(|c| c.set(self.0));
        WINDOW.with(|c| c.set(self.1));
        PHASE.with(|c| c.set(self.2));
    }
}

/// Publishes `window` + `cx` for the duration of `f`.
///
/// Takes `&mut` for the whole call so the borrow checker guarantees the caller
/// cannot touch them while the raw pointers are live: exactly one live `&mut`
/// exists at any instant and the reborrow is well-nested.
pub fn with_published<R>(
    window: &mut Window,
    cx: &mut App,
    phase: Phase,
    f: impl FnOnce() -> R,
) -> R {
    debug_assert!(matches!(phase, Phase::Commit | Phase::App));
    let _g = Guard(
        APP.with(|c| c.replace(cx as *mut App)),
        WINDOW.with(|c| c.replace(window as *mut Window)),
        PHASE.with(|c| c.replace(phase)),
    );
    f()
}

/// [`with_published`] without a window — `Phase::App` only.
pub fn with_published_app<R>(cx: &mut App, f: impl FnOnce() -> R) -> R {
    let _g = Guard(
        APP.with(|c| c.replace(cx as *mut App)),
        WINDOW.with(|c| c.replace(std::ptr::null_mut())),
        PHASE.with(|c| c.replace(Phase::App)),
    );
    f()
}

/// The commit-phase accessor: full `&mut Window` + `&mut App`. `None` when the
/// thread is not inside a `Phase::Commit` publication — throw, don't fall back.
#[inline]
pub fn with_commit<R>(f: impl FnOnce(&mut Window, &mut App) -> R) -> Option<R> {
    if PHASE.with(|c| c.get()) != Phase::Commit {
        return None;
    }
    let (a, w) = (APP.with(|c| c.get()), WINDOW.with(|c| c.get()));
    debug_assert!(!a.is_null() && !w.is_null());
    // SAFETY: non-null only inside `with_published`, which holds the `&mut`s
    // for the whole scope; single-threaded (VM thread == gpui main thread).
    Some(f(unsafe { &mut *w }, unsafe { &mut *a }))
}

/// App-level accessor: works in `Phase::App` and `Phase::Commit`.
#[inline]
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    if !matches!(PHASE.with(|c| c.get()), Phase::App | Phase::Commit) {
        return None;
    }
    let a = APP.with(|c| c.get());
    debug_assert!(!a.is_null());
    // SAFETY: see `with_commit`.
    Some(f(unsafe { &mut *a }))
}

/// Chains onto the default panic hook so a shielded host-fn panic still keeps
/// its message + location for [`panic_to_js`]. Call once at startup.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic>".to_string());
        let loc = info.location().map(|l| l.to_string()).unwrap_or_default();
        LAST_PANIC.with(|c| c.set(Some((msg, loc))));
        prev(info);
    }));
}

/// Throws a JS error (`kind`: 0 Error, 1 TypeError, 2 RangeError) on `g` and
/// returns the `u64` a host fn should return.
pub fn throw_error(g: *mut c_void, kind: u32, msg: &str) -> u64 {
    let utf16: Vec<u16> = msg.encode_utf16().collect();
    let mut err = 0u64;
    unsafe {
        if redjs::ffi::redjs_error_create(g, kind, utf16.as_ptr(), utf16.len(), &mut err) == 0 {
            redjs::ffi::redjs_throw(g, err);
        }
    }
    redjs::api::Value::undefined().0
}

/// The `host_fn!` catch arm: converts a caught panic payload into a thrown JS
/// `Error` and poisons the runtime (I9 — gpui state cannot be trusted after an
/// unwound host fn; the pump checks `poisoned()` and stops scheduling JS).
pub fn panic_to_js(g: *mut c_void, payload: Box<dyn Any + Send>) -> u64 {
    poison();
    let (msg, loc) = LAST_PANIC.with(|c| c.take()).unwrap_or_default();
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or(msg);
    throw_error(g, 0, &format!("native panic at {loc}: {text}"))
}
