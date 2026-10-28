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

/// An uncaught JS error, already extracted into owned strings (raw exception
/// values must never outlive the call that surfaced them — GC contract).
#[derive(Debug, Clone)]
pub struct JsError {
    /// `Name: message` — JSC stacks carry no header line, so this must be
    /// printed alongside the stack or the "what" is silently lost.
    pub header: String,
    pub stack: String,
}

/// The on-disk root a content-provider miss is resolved against: a device-
/// namespace name no volume can carry, so the read fails and the miss is a 404
/// (see `JsRuntime::init`).
const NO_DISK_ROOT: &str = r"\\?\vue-native-no-disk-root";

/// What `navigator.userAgent` says in a vue-native app. Set here, explicitly,
/// rather than left to redjs's default (which happens to be the same string
/// today): it is a product decision, and the one place that makes it is the
/// one place that should change it. `appVersion` and `platform` follow it.
pub const VN_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) RedVue/1.0";

/// Process-wide and read per access, so it holds for every context — a
/// worker's included — whenever it is called.
fn apply_user_agent() {
    redjs::webapi::navigator::set_user_agent(Some(VN_USER_AGENT));
}

/// How long one frame's pump may spend delivering results that arrive from
/// another thread — today IndexedDB's (m25 indexeddb-design §3.3), later a
/// worker's messages, all under this ONE budget.
///
/// A browser runs tasks continuously between frames; this pump runs once per
/// frame. Picking results up once per pump would make every *awaited* request
/// cost a frame — `for (const k of keys) await store.get(k)` over 1000 keys
/// would take ~6 s at 165 Hz. So while a request is in flight the pump keeps
/// delivering, and waits on the server's next result, for at most this long:
/// 1000 sequential awaited gets then take 7–10 frames (redjs
/// tests/idb_libs.rs). The wait only happens while something is outstanding;
/// an idle frame pays one atomic load.
///
/// 4 ms is the design's number: a quarter of a 60 Hz frame, and what a long
/// chain of awaited requests costs a browser's main thread anyway. It is the
/// cap, not the typical cost — a dependent `get` takes ~30 µs, so the usual
/// frame spends a fraction of it. Set here, explicitly, for the same reason as
/// [`VN_USER_AGENT`]: the frame belongs to vue-native, so the share it gives
/// away is vue-native's decision, made in one place.
pub const VN_PUMP_DELIVERY_BUDGET_US: u32 = 4000;

fn apply_pump_budget() {
    redjs::web::set_pump_budget_us(VN_PUMP_DELIVERY_BUDGET_US);
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
        apply_user_agent();
        apply_pump_budget();
        let ctx = Context::new();
        unsafe {
            let rc = ffi::redjs_web_install(ctx.raw());
            assert_eq!(rc, 0, "redjs_web_install failed: {rc}");
            // Relative reads come through the content provider (the app root
            // in dev, the payload in a packaged exe — `set_content_provider`).
            // The on-disk roots below are only what a provider MISS falls
            // through to, and they used to be EMPTY — which made redjs read a
            // miss relative to the PROCESS CWD: a packaged exe double-clicked
            // from Explorer (cwd = its own folder) could `fetch()` files lying
            // beside it, the same exe started from a terminal could not, and a
            // dev host started from the repo could read the repo. A root that
            // cannot exist turns every miss into the plain 404 a browser's
            // server would give, in every mode (m25, docs/PACKAGING.md).
            // Absolute paths and `file:///` URLs are unaffected.
            let root: Vec<u16> = NO_DISK_ROOT.encode_utf16().collect();
            let rc = ffi::redjs_webshim_install(ctx.raw(), root.as_ptr(), root.len(), root.as_ptr(), root.len());
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

    /// Installs the host content provider consulted by `fetch()` and image
    /// loads BEFORE the on-disk roots — the seam a shipped exe uses to serve
    /// its assets straight out of the binary (research/reports/vue-renderer.md
    /// §e.4).
    ///
    /// `f` receives a content-relative UTF-8 key with query/hash stripped,
    /// separators normalized to `/`, and any leading `./` or `/` trimmed
    /// (redjs `webapi::fsutil::clean_relative`), so `fetch("/assets/a.png")`,
    /// `fetch("./assets/a.png")` and `fetch("assets/a.png")` all arrive as
    /// `assets/a.png`. `None` = miss: a 404, because the on-disk roots it
    /// falls through to are a root that cannot exist ([`NO_DISK_ROOT`]) rather
    /// than the process cwd it used to be.
    ///
    /// ## Why `Send + Sync` and not the plain `Fn` the rest of this crate uses
    ///
    /// The raw ABI is a bare `extern "C" fn` **with no user-data pointer**
    /// (`ContentReadFn = fn(*const u8, usize, ContentSinkFn, *mut c_void)
    /// -> i32`; the `*mut c_void` belongs to the *sink*, not to us), so the
    /// closure cannot ride along on the call — it has to live in a process
    /// global that a trampoline reaches. redjs also documents reads as
    /// arriving from any JS thread. Hence: one provider per process, set once.
    /// A second call is a no-op with an advisory rather than a silent swap.
    pub fn set_content_provider(&self, f: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static) {
        if PROVIDER.set(Box::new(f)).is_err() {
            eprintln!("[vn-runtime] set_content_provider called twice — keeping the first provider");
            return;
        }
        unsafe { ffi::redjs_content_provider_set(self.ctx.raw(), content_read as *const c_void) };
    }

    /// Unregisters the provider for this context; local reads go back to the
    /// on-disk roots. (The closure itself stays alive — see the note above.)
    pub fn clear_content_provider(&self) {
        unsafe { ffi::redjs_content_provider_set(self.ctx.raw(), std::ptr::null()) };
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

// --- host content provider --------------------------------------------------

type Provider = Box<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;
static PROVIDER: OnceLock<Provider> = OnceLock::new();

/// ABI tripwire: `redjs_content_provider_set` takes an untyped `*const c_void`,
/// so this is the only thing that makes a signature drift in redjs a compile
/// error here instead of a stack-corrupting call at runtime.
const _: ContentReadFn = content_read;

/// The C-ABI trampoline handed to `redjs_content_provider_set`. Returns 0 after
/// calling `sink` exactly once (hit) or non-zero (miss). Panic-shielded: a
/// panic here would unwind through redjs' `fetch` implementation, so it is
/// converted into a miss instead.
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
            // Contract: exactly one sink call with the FULL bytes. Empty is a
            // legitimate hit (an empty file), and redjs' sink no-ops on len 0.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The product's user agent is the one `navigator` reports — through the
    /// same redjs install the app runs, not by asserting on the constant alone.
    #[test]
    fn navigator_reports_the_vue_native_user_agent() {
        assert_eq!(VN_USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) RedVue/1.0");
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
        assert_eq!(read("navigator.appVersion"), "5.0 (Windows NT 10.0; Win64; x64) RedVue/1.0");
        assert_eq!(read("navigator.platform"), "Win32");
        assert_eq!(read("String(navigator instanceof Navigator)"), "true");
    }

    /// The vue-native build carries IndexedDB, and a chain of awaited requests
    /// completes inside the pump the app runs once per frame — TIMERS | RAF |
    /// MICROTASKS under the lock, `JsRuntime::pump`'s shape — in a handful of
    /// frames, because the delivery budget lets results land mid-pump.
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
