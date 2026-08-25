//! The process-lifetime JSC context: boot, eval, per-frame pump, and the
//! uncaught-error sink.

use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;

use redjs::api::{Context, Error, Value};
use redjs::ffi::{self, RjsPumpStats};

pub use redjs::web::{PUMP_MICROTASKS, PUMP_RAF, PUMP_TIMERS};

/// An uncaught JS error, already extracted into owned strings (raw exception
/// values must never outlive the call that surfaced them — GC contract).
#[derive(Debug, Clone)]
pub struct JsError {
    /// `Name: message` — JSC stacks carry no header line, so this must be
    /// printed alongside the stack or the "what" is silently lost.
    pub header: String,
    pub stack: String,
}

thread_local! {
    static RUNTIME: OnceCell<JsRuntime> = const { OnceCell::new() };
    static ERRORS: RefCell<VecDeque<JsError>> = const { RefCell::new(VecDeque::new()) };
}

/// One VM + global object, created once, never released. `!Send` by
/// construction (holds `redjs::api::Context`); lives in a thread-local so it
/// can only ever be touched from the thread that created it.
pub struct JsRuntime {
    ctx: Context,
}

impl JsRuntime {
    /// Boots JSC and installs the web platform layer. Call once, on the gpui
    /// main thread, before anything else touches redjs.
    pub fn init() {
        crate::bridge::install_panic_hook();
        unsafe {
            // MUST go through ffi::redjs_init (not Context::new alone): it
            // anchors the ICU zstd-decompress hook that the thin -bundle
            // archive otherwise drops (research/reports/redjs.md §a.4).
            let rc = ffi::redjs_init(std::ptr::null());
            assert_eq!(rc, 0, "redjs_init failed: {rc}");
        }
        let ctx = Context::new();
        unsafe {
            let rc = ffi::redjs_web_install(ctx.raw());
            assert_eq!(rc, 0, "redjs_web_install failed: {rc}");
            // Scripts/assets roots empty for now — assets will come through
            // redjs_content_provider_set later.
            let rc = ffi::redjs_webshim_install(ctx.raw(), std::ptr::null(), 0, std::ptr::null(), 0);
            assert_eq!(rc, 0, "redjs_webshim_install failed: {rc}");
            ffi::redjs_set_uncaught_handler(on_uncaught as *const c_void);
        }
        RUNTIME.with(|r| {
            r.set(JsRuntime { ctx })
                .unwrap_or_else(|_| panic!("JsRuntime::init called twice"))
        });
    }

    /// Runs `f` with the runtime. Panics if [`JsRuntime::init`] has not run on
    /// this thread.
    pub fn with<R>(f: impl FnOnce(&JsRuntime) -> R) -> R {
        RUNTIME.with(|r| f(r.get().expect("JsRuntime::init has not run on this thread")))
    }

    pub fn raw(&self) -> *mut c_void {
        self.ctx.raw()
    }

    pub fn ctx(&self) -> &Context {
        &self.ctx
    }

    /// Evaluates a classic script; `url` shows up in JS stack traces.
    pub fn eval(&self, src: &str, url: &str) -> Result<(), JsError> {
        match self.ctx.eval_named(src, url) {
            Ok(_) => Ok(()),
            Err(Error::Exception(e)) => Err(self.describe(e)),
            Err(e) => Err(JsError { header: format!("engine error: {e}"), stack: String::new() }),
        }
    }

    /// Registers a raw `extern "sysv64"` host function (written with
    /// [`crate::host_fn!`]) as a global.
    pub fn register(&self, name: &str, arity: u32, f: *const c_void) {
        let v = unsafe { self.ctx.function_sysv(name, arity, f) }
            .unwrap_or_else(|e| panic!("registering host fn {name}: {e:?}"));
        self.ctx
            .global_set(name, v)
            .unwrap_or_else(|e| panic!("defining global {name}: {e:?}"));
    }

    /// One frame of the web scheduler: native task drain → due timers → rAF
    /// batch → idle callbacks → microtasks. Must run on the VM thread; the
    /// caller decides what phase (if any) is published around it.
    pub fn pump(&self, flags: u32) -> RjsPumpStats {
        let mut stats = RjsPumpStats {
            now: 0.0,
            timers_fired: 0,
            raf_fired: 0,
            raf_pending: 0,
            timers_active: 0,
            next_timer_deadline: -1.0,
        };
        unsafe {
            ffi::redjs_lock(self.ctx.raw());
            ffi::redjs_pump(self.ctx.raw(), flags, &mut stats);
            ffi::redjs_unlock(self.ctx.raw());
        }
        stats
    }

    /// Uncaught errors accumulated since the last drain (pump-callback throws,
    /// unhandled promise rejections). Drain once per pump.
    pub fn drain_errors(&self) -> Vec<JsError> {
        ERRORS.with(|e| e.borrow_mut().drain(..).collect())
    }

    /// `Name: message` + stack for an exception value, extracted immediately
    /// into owned strings.
    pub fn describe(&self, exc: Value) -> JsError {
        let (msg, stack) = self.ctx.error_info(exc);
        let name = self
            .ctx
            .get(exc, &self.ctx.atom("name"))
            .ok()
            .and_then(|v| self.ctx.read_string(v).ok())
            .unwrap_or_default();
        let header = match (name.is_empty(), msg.is_empty()) {
            (false, false) => format!("{name}: {msg}"),
            (false, true) => name,
            (true, false) => msg,
            (true, true) => self.ctx.to_rust_string(exc).unwrap_or_default(),
        };
        JsError { header, stack }
    }
}

/// Process-global uncaught handler (MS ABI, called with the VM lock held,
/// possibly re-entrantly from inside our own pump). A pure sink by contract:
/// extract strings, enqueue, return. Never calls back into JS or gpui.
unsafe extern "C" fn on_uncaught(g: *mut c_void, exc: u64) {
    let _ = std::panic::catch_unwind(|| {
        let err = describe_raw(g, exc);
        ERRORS.with(|e| e.borrow_mut().push_back(err));
    });
}

fn describe_raw(g: *mut c_void, exc: u64) -> JsError {
    unsafe {
        let (mut msg_v, mut stack_v) = (0u64, 0u64);
        ffi::redjs_error_info(g, exc, &mut msg_v, &mut stack_v);
        let msg = redjs::webapi::util::read_string(g, msg_v).unwrap_or_default();
        let stack = redjs::webapi::util::read_string(g, stack_v).unwrap_or_default();
        let name_v = redjs::webapi::util::get(g, exc, "name");
        let name = redjs::webapi::util::read_string(g, name_v).unwrap_or_default();
        let header = if name.is_empty() { msg } else { format!("{name}: {msg}") };
        JsError { header, stack }
    }
}
