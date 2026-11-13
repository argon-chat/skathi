//! The process-lifetime JSC context: boot, eval, per-frame pump, and the
//! uncaught-error sink.

use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::OnceLock;

use redjs::api::{Context, Error, Value};
use redjs::ffi::{self, RjsPumpStats};
use redjs::webapi::fsutil::{ContentReadFn, ContentSinkFn};

pub use redjs::web::{PUMP_MICROTASKS, PUMP_RAF, PUMP_TIMERS};

/// An uncaught JS error, extracted into owned strings (raw exception values must not outlive the call).
#[derive(Debug, Clone)]
pub struct JsError {
    /// `Name: message`; JSC stacks carry no header line, so print it alongside the stack.
    pub header: String,
    pub stack: String,
}

/// The disk root a content-provider miss resolves against: a name no volume can carry, so a miss
/// is a 404 instead of a read relative to the process cwd.
const NO_DISK_ROOT: &str = r"\\?\vue-native-no-disk-root";

/// `navigator.userAgent` (and `appVersion`/`platform`): redjs's OS-specific identity, set explicitly.
pub const VN_USER_AGENT: &str = redjs::webapi::navigator::DEFAULT_USER_AGENT;

/// Process-wide, so it holds for every context, workers included.
fn apply_user_agent() {
    redjs::webapi::navigator::set_user_agent(Some(VN_USER_AGENT));
}

/// Cap on how long one pump may spend delivering results that arrive from other threads
/// (IndexedDB today). Picking results up once per pump would cost every awaited request a frame;
/// while a request is in flight the pump keeps delivering, for at most this long. The actual
/// per-frame budget is [`delivery_budget_us`].
pub const VN_PUMP_DELIVERY_BUDGET_US: u32 = 4000;

/// Floor under [`delivery_budget_us`]: below this the thread hop and the pump's spin dominate the slice.
pub const VN_PUMP_DELIVERY_MIN_US: u32 = 500;

/// The per-pump delivery budget for a display refreshing every `frame_ms`: a quarter of the frame,
/// between [`VN_PUMP_DELIVERY_MIN_US`] and [`VN_PUMP_DELIVERY_BUDGET_US`]; the cap when unknown.
///
/// Pump time for reads does not depend on the budget, only the number of frames it is spread over,
/// so a fixed cap would take a larger share of each frame on high-refresh displays.
pub fn delivery_budget_us(frame_ms: Option<f32>) -> u32 {
    match frame_ms {
        Some(ms) if ms.is_finite() && ms > 0.0 => {
            ((ms * 250.0) as u32).clamp(VN_PUMP_DELIVERY_MIN_US, VN_PUMP_DELIVERY_BUDGET_US)
        }
        _ => VN_PUMP_DELIVERY_BUDGET_US,
    }
}

fn apply_pump_budget() {
    redjs::web::set_pump_budget_us(VN_PUMP_DELIVERY_BUDGET_US);
}

/// The JavaScriptCore options every app runs with, applied by `redjs_init` before the first VM.
///
/// `jitPolicyScale=0.5` halves the counts after which JSC tiers a function up, so short
/// interactions (Vue patch and mount code run a few hundred times) leave the interpreter sooner.
/// An option set in the environment (`JSC_jitPolicyScale=1`) wins: it is read first and this list
/// is applied after, so it is filtered out in [`jsc_options`].
pub const VN_JSC_OPTIONS: &str = "jitPolicyScale=0.5";

/// [`VN_JSC_OPTIONS`], minus the options the environment already sets.
fn jsc_options() -> std::ffi::CString {
    let kept: Vec<&str> = VN_JSC_OPTIONS
        .split(',')
        .filter(|opt| {
            let name = opt.split('=').next().unwrap_or(opt);
            std::env::var_os(format!("JSC_{name}")).is_none()
        })
        .collect();
    std::ffi::CString::new(kept.join(",")).expect("VN_JSC_OPTIONS holds no NUL")
}

/// Re-derives the delivery budget from the display's refresh interval (`None` until the platform
/// says). The host calls it once per frame, before the pump.
pub fn set_frame_budget_ms(frame_ms: Option<f32>) {
    let us = delivery_budget_us(frame_ms);
    if redjs::web::pump_budget_us() != us {
        redjs::web::set_pump_budget_us(us);
    }
}

thread_local! {
    static RUNTIME: OnceCell<JsRuntime> = const { OnceCell::new() };
    static ERRORS: RefCell<VecDeque<JsError>> = const { RefCell::new(VecDeque::new()) };
}

/// One VM + global object, created once, never released. `!Send`; lives in a thread-local so it is
/// only touched from the thread that created it.
pub struct JsRuntime {
    ctx: Context,
}

impl JsRuntime {
    /// Boots JSC and installs the web platform layer. Call once, on the gpui main thread.
    ///
    /// Once per PROCESS: a second boot reapplies [`VN_JSC_OPTIONS`] after the first VM froze them and
    /// JSC fast-fails (`0xC0000409`), so it panics instead. Tests use [`crate::testing::on_js_thread`].
    pub fn init() {
        static BOOTED: OnceLock<std::thread::ThreadId> = OnceLock::new();
        let here = std::thread::current().id();
        if BOOTED.set(here).is_err() {
            panic!(
                "JsRuntime::init called a second time in this process (first on {:?}, now on {:?}). \
                 The runtime boots once per process, and a second boot aborts it. A test that needs \
                 JS runs its body through vn_runtime::testing::on_js_thread instead.",
                BOOTED.get().expect("set"),
                here
            );
        }
        crate::bridge::install_panic_hook();
        unsafe {
            // Must go through ffi::redjs_init: it anchors the ICU zstd-decompress hook that the thin
            // -bundle archive otherwise drops.
            let options = jsc_options();
            let rc = ffi::redjs_init(options.as_ptr());
            assert_eq!(rc, 0, "redjs_init failed: {rc}");
        }
        apply_user_agent();
        apply_pump_budget();
        let ctx = Context::new();
        unsafe {
            let rc = ffi::redjs_web_install(ctx.raw());
            assert_eq!(rc, 0, "redjs_web_install failed: {rc}");
            // Relative reads go through the content provider; these roots are only where a miss
            // falls through to. Empty roots made a miss read relative to the process cwd, so a
            // root that cannot exist turns every miss into a plain 404. Absolute paths and
            // `file:///` URLs are unaffected.
            let root: Vec<u16> = NO_DISK_ROOT.encode_utf16().collect();
            let rc = ffi::redjs_webshim_install(ctx.raw(), root.as_ptr(), root.len(), root.as_ptr(), root.len());
            assert_eq!(rc, 0, "redjs_webshim_install failed: {rc}");
            ffi::redjs_set_uncaught_handler(on_uncaught as *const c_void);
        }
        RUNTIME.with(|r| r.set(JsRuntime { ctx }).unwrap_or_else(|_| unreachable!("BOOTED admits one init")));
    }

    /// Runs `f` with the runtime. Panics if [`JsRuntime::init`] has not run on this thread.
    pub fn with<R>(f: impl FnOnce(&JsRuntime) -> R) -> R {
        RUNTIME.with(|r| f(r.get().expect("JsRuntime::init has not run on this thread")))
    }

    /// [`JsRuntime::with`] for callers that can do without: `None` when no runtime runs on this
    /// thread. vn-tree resolves `blob:` URLs through this.
    pub fn try_with<R>(f: impl FnOnce(&JsRuntime) -> R) -> Option<R> {
        RUNTIME.with(|r| r.get().map(f))
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

    /// Registers a raw host function (written with `raw_host_fn!` or
    /// [`crate::host_fn!`]) as a global.
    pub fn register(&self, name: &str, arity: u32, f: *const c_void) {
        let v = unsafe { self.ctx.function_sysv(name, arity, f) }
            .unwrap_or_else(|e| panic!("registering host fn {name}: {e:?}"));
        self.ctx
            .global_set(name, v)
            .unwrap_or_else(|e| panic!("defining global {name}: {e:?}"));
    }

    /// One frame of the web scheduler: native task drain, due timers, rAF, idle callbacks,
    /// microtasks. Must run on the VM thread; the caller decides which phase is published around it.
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

    /// Uncaught errors accumulated since the last drain (pump throws, unhandled rejections).
    pub fn drain_errors(&self) -> Vec<JsError> {
        ERRORS.with(|e| e.borrow_mut().drain(..).collect())
    }

    /// Installs the host content provider consulted by `fetch()` and image loads before the disk
    /// roots; a shipped exe serves its assets from the binary through it.
    ///
    /// `f` receives a content-relative UTF-8 key (query/hash stripped, `/` separators, leading
    /// `./` or `/` trimmed); `None` is a miss, a 404.
    ///
    /// `Send + Sync`: the raw ABI is a bare `extern "C" fn` with no user-data pointer, so the closure
    /// lives in a process global reached by a trampoline, and redjs may read from any JS thread.
    /// One provider per process; a second call is a no-op with an advisory.
    pub fn set_content_provider(&self, f: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static) {
        if PROVIDER.set(Box::new(f)).is_err() {
            eprintln!("[vn-runtime] set_content_provider called twice — keeping the first provider");
            return;
        }
        unsafe { ffi::redjs_content_provider_set(self.ctx.raw(), content_read as *const c_void) };
    }

    /// Unregisters the provider for this context; local reads go back to the disk roots.
    pub fn clear_content_provider(&self) {
        unsafe { ffi::redjs_content_provider_set(self.ctx.raw(), std::ptr::null()) };
    }

    /// `Name: message` + stack for an exception value, extracted into owned strings.
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

type Provider = Box<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;
static PROVIDER: OnceLock<Provider> = OnceLock::new();

/// ABI tripwire: `redjs_content_provider_set` takes an untyped pointer, so a signature drift in
/// redjs becomes a compile error here.
const _: ContentReadFn = content_read;

/// C-ABI trampoline for `redjs_content_provider_set`: 0 after calling `sink` once (hit), non-zero
/// on a miss. A panic is converted into a miss rather than unwinding through redjs.
unsafe extern "C" fn content_read(
    path_ptr: *const u8,
    path_len: usize,
    sink: ContentSinkFn,
    sink_ctx: *mut c_void,
) -> i32 {
    const MISS: i32 = 1;
    let Some(provider) = PROVIDER.get() else { return MISS };
    if path_ptr.is_null() {
        return MISS;
    }
    let bytes = core::slice::from_raw_parts(path_ptr, path_len);
    let Ok(key) = std::str::from_utf8(bytes) else { return MISS };
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider(key)));
    match found {
        Ok(Some(data)) => {
            // Exactly one sink call with the full bytes; empty is a hit (an empty file).
            sink(sink_ctx, data.as_ptr(), data.len());
            0
        }
        Ok(None) => MISS,
        Err(_) => {
            eprintln!("[vn-runtime] content provider panicked for {key:?} — treated as a miss");
            MISS
        }
    }
}

/// Process-global uncaught handler, called with the VM lock held, possibly re-entrantly from our
/// own pump. A pure sink: extract strings, enqueue, return.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The product user agent is what `navigator` reports through the real redjs install.
    #[test]
    fn navigator_reports_the_vue_native_user_agent() {
        assert_eq!(VN_USER_AGENT, redjs::webapi::navigator::DEFAULT_USER_AGENT);
        unsafe { assert_eq!(ffi::redjs_init(std::ptr::null()), 0) };
        apply_user_agent();
        let ctx = Context::new();
        unsafe {
            assert_eq!(ffi::redjs_web_install(ctx.raw()), 0);
            let root: Vec<u16> = NO_DISK_ROOT.encode_utf16().collect();
            assert_eq!(ffi::redjs_webshim_install(ctx.raw(), root.as_ptr(), root.len(), root.as_ptr(), root.len()), 0);
        }
        let read = |src: &str| ctx.to_rust_string(ctx.eval(src).unwrap()).unwrap();
        assert_eq!(read("navigator.userAgent"), VN_USER_AGENT);
        assert_eq!(read("navigator.appVersion"), redjs::webapi::navigator::app_version(VN_USER_AGENT));
        assert_eq!(read("navigator.platform"), redjs::webapi::navigator::platform(VN_USER_AGENT));
        #[cfg(target_os = "macos")]
        assert_eq!(read("navigator.platform"), "MacIntel");
        #[cfg(target_os = "windows")]
        assert_eq!(read("navigator.platform"), "Win32");
        assert_eq!(read("String(navigator instanceof Navigator)"), "true");
    }

    /// A quarter of the frame, capped at 4 ms and floored at 0.5 ms; the cap when unknown.
    #[test]
    fn the_delivery_budget_is_a_quarter_of_the_frame() {
        assert_eq!(delivery_budget_us(None), VN_PUMP_DELIVERY_BUDGET_US);
        assert_eq!(delivery_budget_us(Some(1000.0 / 60.0)), 4000, "60 Hz: the cap (a quarter is 4.17 ms)");
        assert_eq!(delivery_budget_us(Some(1000.0 / 120.0)), 2083);
        assert_eq!(delivery_budget_us(Some(1000.0 / 144.0)), 1736);
        assert_eq!(delivery_budget_us(Some(1000.0 / 240.0)), 1041);
        assert_eq!(delivery_budget_us(Some(1000.0 / 1000.0)), VN_PUMP_DELIVERY_MIN_US);
        assert_eq!(delivery_budget_us(Some(100.0)), VN_PUMP_DELIVERY_BUDGET_US, "30 Hz and slower: still the cap");
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(delivery_budget_us(Some(bad)), VN_PUMP_DELIVERY_BUDGET_US, "{bad}");
        }
    }

    /// Awaited IndexedDB requests complete in a handful of frames, because results land mid-pump.
    #[cfg(feature = "indexeddb")]
    #[test]
    fn indexeddb_answers_an_awaited_chain_within_a_few_frames() {
        unsafe { assert_eq!(ffi::redjs_init(std::ptr::null()), 0) };
        apply_pump_budget();
        assert_eq!(redjs::web::pump_budget_us(), VN_PUMP_DELIVERY_BUDGET_US);
        let ctx = Context::new();
        unsafe {
            assert_eq!(ffi::redjs_web_install(ctx.raw()), 0);
            let root: Vec<u16> = NO_DISK_ROOT.encode_utf16().collect();
            assert_eq!(ffi::redjs_webshim_install(ctx.raw(), root.as_ptr(), root.len(), root.as_ptr(), root.len()), 0);
        }
        let read = |src: &str| ctx.to_rust_string(ctx.eval(src).unwrap()).unwrap();
        assert_eq!(read("typeof indexedDB + ',' + typeof IDBKeyRange + ',' + typeof navigator.storage.estimate"), "object,function,function");
        read(
            "globalThis.__r = '';
             const p = (q) => new Promise((res, rej) => { q.onsuccess = () => res(q.result); q.onerror = () => rej(q.error); });
             (async () => {
               const open = indexedDB.open('vn-runtime-chain', 1);
               open.onupgradeneeded = () => open.result.createObjectStore('s');
               const db = await p(open);
               const tx = db.transaction('s', 'readwrite');
               for (let i = 0; i < 200; i++) tx.objectStore('s').put(i * 2, i);
               await new Promise((res) => { tx.oncomplete = res; });
               let sum = 0;
               for (let i = 0; i < 200; i++) sum += await p(db.transaction('s').objectStore('s').get(i));
               return sum;
             })().then((v) => { globalThis.__r = String(v); }, (e) => { globalThis.__r = 'ERR ' + e; });
             ''",
        );
        let mut frames = 0;
        let out = loop {
            unsafe {
                ffi::redjs_lock(ctx.raw());
                ffi::redjs_pump(ctx.raw(), PUMP_TIMERS | PUMP_RAF | PUMP_MICROTASKS, std::ptr::null_mut());
                ffi::redjs_unlock(ctx.raw());
            }
            frames += 1;
            let r = read("globalThis.__r");
            if !r.is_empty() || frames > 500 {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(out, "39800");
        assert!(frames <= 10, "200 awaited gets took {frames} frames");
    }
}
