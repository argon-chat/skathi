//! The JS↔gpui call boundary: raw `&mut App` / `&mut Window` pointers are
//! published for a bounded scope with an RAII guard, tagged with a [`Phase`],
//! and host functions may only reach them through [`with_commit`] /
//! [`with_app`]. Outside a published scope those return `None` and the caller
//! must throw a JS error rather than touch gpui.
//!
//! Teardown lives in `Drop` (as gpui's `ElementArenaScope`), so a panic cannot leave a dangling pointer published.

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
    /// Why the runtime was poisoned: the first caught panic's message and
    /// location, for the host's crash dialog (vn-app src/crash_window.rs).
    static POISON_REASON: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static DRAIN_OWED: Cell<bool> = const { Cell::new(false) };
}

/// A dispatch made under a gpui listener leaves its microtask checkpoint owed instead of running
/// it there; every JS entry settles it first ([`settle_owed_drain`]), so there is still a
/// checkpoint between any two events. Call it BEFORE the dispatch's call into JS.
///
/// It takes the engine's API lock and keeps it until settled: JSC drains microtasks whenever the
/// outermost API lock is released.
///
/// Returns `false`, and owes nothing, when JS is already running below the dispatch (a listener run
/// by a forced layout's draw from inside a script): that dispatch is nested and the enclosing
/// script's checkpoint covers it. Owing there would keep a lock past the release of heap access,
/// and JSC aborts ("Attempting to release access but the mutator does not have access").
pub fn owe_drain() -> bool {
    if DRAIN_OWED.with(|c| c.get()) {
        return true;
    }
    let owed = crate::JsRuntime::try_with(|rt| {
        let g = rt.raw();
        // SAFETY: the context is live and this is its VM thread; `rjs_in_js` runs under the lock
        // just taken, so with JS on the stack the unlock only counts down.
        unsafe {
            redjs::ffi::redjs_lock(g);
            if redjs::glue::rjs_in_js(g) != 0 {
                redjs::ffi::redjs_unlock(g);
                return false;
            }
        }
        true
    })
    .unwrap_or(false);
    DRAIN_OWED.with(|c| c.set(owed));
    owed
}

/// Is a checkpoint owed ([`owe_drain`])?
pub fn drain_owed() -> bool {
    DRAIN_OWED.with(|c| c.get())
}

/// The owed checkpoint, now, if there is one, and the API lock released.
/// Every publication runs it first ([`with_published`],
/// [`with_published_app`]); so must a host that calls into JS outside one.
pub fn settle_owed_drain() {
    if !DRAIN_OWED.with(|c| c.replace(false)) {
        return;
    }
    crate::JsRuntime::try_with(|rt| {
        if !poisoned() {
            rt.ctx().drain_microtasks();
        }
        // SAFETY: as in `owe_drain`, which took the lock this releases.
        unsafe { redjs::ffi::redjs_unlock(rt.raw()) };
    });
}

/// True once a host-fn panic has been caught. gpui state must be assumed
/// corrupt (leases dropped, update depth unbalanced): stop scheduling JS.
pub fn poisoned() -> bool {
    POISONED.with(|c| c.get())
}

pub fn poison() {
    POISONED.with(|c| c.set(true));
}

/// Why the runtime is poisoned: `"<message> at <file:line:col>"` of the first
/// caught panic, or `None` when it is not poisoned or was poisoned by a bare
/// [`poison`] call that gave no reason.
pub fn poison_reason() -> Option<String> {
    POISON_REASON.with(|r| r.borrow().clone())
}

/// [`poison`], keeping the first reason given: the panic that broke the
/// runtime, not one that followed from it.
pub fn poison_with(reason: String) {
    poison();
    POISON_REASON.with(|r| {
        let mut r = r.borrow_mut();
        if r.is_none() {
            *r = Some(reason);
        }
    });
}

/// `"<message> at <location>"`, or the message alone without a location.
pub fn describe_panic(message: &str, location: &str) -> String {
    if location.is_empty() { message.to_string() } else { format!("{message} at {location}") }
}

struct Guard(*mut App, *mut Window, Phase);

impl Drop for Guard {
    fn drop(&mut self) {
        APP.with(|c| c.set(self.0));
        WINDOW.with(|c| c.set(self.1));
        PHASE.with(|c| c.set(self.2));
    }
}

/// Publishes `window` + `cx` for the duration of `f`, after settling an owed checkpoint.
///
/// Takes `&mut` for the whole call so the caller cannot touch them while the raw pointers are live.
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
    settle_owed_drain();
    f()
}

/// [`with_published`] without a window — `Phase::App` only.
pub fn with_published_app<R>(cx: &mut App, f: impl FnOnce() -> R) -> R {
    let _g = Guard(
        APP.with(|c| c.replace(cx as *mut App)),
        WINDOW.with(|c| c.replace(std::ptr::null_mut())),
        PHASE.with(|c| c.replace(Phase::App)),
    );
    settle_owed_drain();
    f()
}

/// Runs `f`, a `Window::draw` made from inside a publication, as [`Phase::Draw`]: nothing is
/// reachable through [`with_commit`] or [`with_app`] until it returns.
///
/// A forced layout draws from inside a script's `Phase::Commit`, whose pointers are the `Window` and
/// `App` gpui is already working through; handing them out again would alias the `&mut`s.
pub fn with_draw<R>(f: impl FnOnce() -> R) -> R {
    let _g = Guard(
        APP.with(|c| c.get()),
        WINDOW.with(|c| c.get()),
        PHASE.with(|c| c.replace(Phase::Draw)),
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

/// The `host_fn!` catch arm: converts a caught panic payload into a thrown JS `Error` and poisons
/// the runtime.
pub fn panic_to_js(g: *mut c_void, payload: Box<dyn Any + Send>) -> u64 {
    let (msg, loc) = LAST_PANIC.with(|c| c.take()).unwrap_or_default();
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or(msg);
    poison_with(describe_panic(&text, &loc));
    throw_error(g, 0, &format!("native panic at {loc}: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JsRuntime;
    use redjs::api::Value;

    /// `owe()` owes the checkpoint as vn-tree's `dispatch_event` does under a gpui listener, and
    /// answers whether it did.
    fn install_owe(rt: &JsRuntime) {
        let f = rt.ctx().function("owe", 0, |_g, _frame| Value::boolean(owe_drain()).0).unwrap();
        rt.ctx().global_set("owe", f).unwrap();
    }

    fn read(rt: &JsRuntime, src: &str) -> String {
        let v = rt.ctx().eval(src).unwrap();
        rt.ctx().to_rust_string(v).unwrap()
    }

    /// Regression: a microtask run by JSC's own drain in `JSLock::willReleaseLock` reached a
    /// listener's dispatch through a forced layout's draw; owing there kept a lock past the heap
    /// access and the next release aborted the process.
    #[test]
    fn a_dispatch_inside_running_js_owes_nothing() {
        crate::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                install_owe(rt);
                // No explicit drain: the reaction runs from the eval's final unlock, the case under test.
                rt.ctx().eval("globalThis.seen = []; Promise.resolve().then(() => seen.push(owe()))").unwrap();
            });
            assert!(!drain_owed(), "a dispatch nested in running JS owes nothing");
            settle_owed_drain();
            JsRuntime::with(|rt| {
                unsafe { redjs::ffi::redjs_gc(rt.raw(), 0) };
                assert_eq!(read(rt, "String(seen)"), "false", "the reaction ran in JSC's drain, and owed nothing");
            });
        });
    }

    /// Outside JS, the dispatch still owes; the settle runs the checkpoint and gives the lock back.
    #[test]
    fn a_dispatch_outside_js_owes_its_checkpoint_until_it_is_settled() {
        crate::testing::on_js_thread(|| {
            assert!(owe_drain());
            assert!(owe_drain(), "owing twice is one owed checkpoint");
            JsRuntime::with(|rt| {
                rt.ctx().eval("globalThis.ran = false; Promise.resolve().then(() => { ran = true; })").unwrap();
                assert_eq!(read(rt, "String(ran)"), "false", "a call's end does not drain while the checkpoint is owed");
            });
            settle_owed_drain();
            assert!(!drain_owed());
            JsRuntime::with(|rt| {
                unsafe { redjs::ffi::redjs_gc(rt.raw(), 0) };
                assert_eq!(read(rt, "String(ran)"), "true", "the settle ran the checkpoint");
            });
        });
    }

    /// A draw made inside a publication hides it, and gives it back.
    #[test]
    fn a_draw_inside_a_publication_freezes_it() {
        std::thread::spawn(|| {
            PHASE.with(|c| c.set(Phase::Commit));
            let inside = with_draw(|| {
                let hidden = with_commit(|_, _| ()).is_none() && with_app(|_| ()).is_none();
                (hidden, PHASE.with(|c| c.get()))
            });
            assert_eq!(inside, (true, Phase::Draw));
            assert_eq!(PHASE.with(|c| c.get()), Phase::Commit);
        })
        .join()
        .unwrap();
    }

    /// A second boot in one process panics with a message saying what to do.
    #[test]
    fn a_second_init_is_refused_with_a_message() {
        let message = crate::testing::on_js_thread(|| {
            let payload = std::panic::catch_unwind(JsRuntime::init).expect_err("a second init panics");
            payload.downcast_ref::<String>().cloned().unwrap_or_default()
        });
        assert!(message.contains("vn_runtime::testing::on_js_thread"), "{message}");
    }

    /// The crash dialog names the panic that broke the runtime, not one that followed from it.
    /// Runs on a thread of its own: the poison is per thread.
    #[test]
    fn the_first_poison_reason_is_kept() {
        std::thread::spawn(|| {
            assert!(!poisoned());
            assert_eq!(poison_reason(), None);
            poison_with(describe_panic("index out of bounds", "crates/x/src/lib.rs:12:5"));
            poison_with(describe_panic("already borrowed", "crates/y/src/lib.rs:1:1"));
            assert!(poisoned());
            assert_eq!(poison_reason().as_deref(), Some("index out of bounds at crates/x/src/lib.rs:12:5"));
        })
        .join()
        .unwrap();
        assert_eq!(describe_panic("boom", ""), "boom", "no location, no dangling `at`");
    }
}
