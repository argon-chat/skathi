//! The process-lifetime JSC context: boot, eval, per-frame pump, and the uncaught-error sink.

use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::OnceLock;

use redjs::api::{Context, Error, Value};
use redjs::ffi::{self, RjsPumpStats};
use redjs::webapi::fsutil::{ContentReadFn, ContentSinkFn};

pub use redjs::web::{PUMP_MICROTASKS, PUMP_RAF, PUMP_TIMERS};

#[derive(Debug, Clone)]
pub struct JsError {
    pub header: String,
    pub stack: String,
}

/// The disk root a content-provider miss resolves against: a name no volume can carry, so a miss is a 404.
const NO_DISK_ROOT: &str = r"\\?\skathi-no-disk-root";

pub const SK_USER_AGENT: &str = redjs::webapi::navigator::DEFAULT_USER_AGENT;

fn apply_user_agent() {
    redjs::webapi::navigator::set_user_agent(Some(SK_USER_AGENT));
}

/// Cap on how long one pump may spend delivering results that arrive from other threads (IndexedDB).
pub const SK_PUMP_DELIVERY_BUDGET_US: u32 = 4000;

pub const SK_PUMP_DELIVERY_MIN_US: u32 = 500;

pub fn delivery_budget_us(frame_ms: Option<f32>) -> u32 {
    match frame_ms {
        Some(ms) if ms.is_finite() && ms > 0.0 => {
            ((ms * 250.0) as u32).clamp(SK_PUMP_DELIVERY_MIN_US, SK_PUMP_DELIVERY_BUDGET_US)
        }
        _ => SK_PUMP_DELIVERY_BUDGET_US,
    }
}

fn apply_pump_budget() {
    redjs::web::set_pump_budget_us(SK_PUMP_DELIVERY_BUDGET_US);
}

/// Gives `--shading`'s `input.time` the zero of `performance.now()`, so a page can hand an effect
/// `performance.now() / 1000` as a start time.
fn share_clock_with_effects() {
    let now = std::time::Instant::now();
    let elapsed = std::time::Duration::from_secs_f64(redjs::web::now_ms().max(0.0) / 1000.0);
    gpui::set_effect_clock_origin(now.checked_sub(elapsed).unwrap_or(now));
}

/// The JavaScriptCore options every app runs with, applied by `redjs_init` before the first VM. An
/// option set in the environment (`JSC_jitPolicyScale=1`) wins, so it is filtered out in [`jsc_options`].
pub const SK_JSC_OPTIONS: &str = "jitPolicyScale=0.5";

fn jsc_options() -> std::ffi::CString {
    let kept: Vec<&str> = SK_JSC_OPTIONS
        .split(',')
        .filter(|opt| {
            let name = opt.split('=').next().unwrap_or(opt);
            std::env::var_os(format!("JSC_{name}")).is_none()
        })
        .collect();
    std::ffi::CString::new(kept.join(",")).expect("SK_JSC_OPTIONS holds no NUL")
}

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

/// One VM + global object, created once, never released. `!Send`: only touched from the thread that created it.
pub struct JsRuntime {
    ctx: Context,
}

impl JsRuntime {
    /// Boots JSC and installs the web platform layer. Once per PROCESS: a second boot reapplies
    /// [`SK_JSC_OPTIONS`] after the first VM froze them and JSC fast-fails, so it panics instead.
    pub fn init() {
        static BOOTED: OnceLock<std::thread::ThreadId> = OnceLock::new();
        let here = std::thread::current().id();
        if BOOTED.set(here).is_err() {
            panic!(
                "JsRuntime::init called a second time in this process (first on {:?}, now on {:?}). \
                 The runtime boots once per process, and a second boot aborts it. A test that needs \
                 JS runs its body through sk_runtime::testing::on_js_thread instead.",
                BOOTED.get().expect("set"),
                here
            );
        }
        crate::bridge::install_panic_hook();
        unsafe {
            // Must go through ffi::redjs_init: it anchors the ICU zstd-decompress hook the thin-bundle archive drops.
            let options = jsc_options();
            let rc = ffi::redjs_init(options.as_ptr());
            assert_eq!(rc, 0, "redjs_init failed: {rc}");
        }
        apply_user_agent();
        apply_pump_budget();
        #[cfg(feature = "webgpu")]
        redjs_webgpu::register();
        let ctx = Context::new();
        unsafe {
            let rc = ffi::redjs_web_install(ctx.raw());
            assert_eq!(rc, 0, "redjs_web_install failed: {rc}");
            share_clock_with_effects();
            // Relative reads go through the content provider; empty roots would make a miss read relative to the
            // process cwd, so a root that cannot exist turns every miss into a 404.
            let root: Vec<u16> = NO_DISK_ROOT.encode_utf16().collect();
            let rc = ffi::redjs_webshim_install(
                ctx.raw(),
                root.as_ptr(),
                root.len(),
                root.as_ptr(),
                root.len(),
            );
            assert_eq!(rc, 0, "redjs_webshim_install failed: {rc}");
            ffi::redjs_set_uncaught_handler(on_uncaught as *const c_void);
            crate::unsupported::install(ctx.raw());
            crate::host::install(ctx.raw());
        }
        RUNTIME.with(|r| {
            r.set(JsRuntime { ctx })
                .unwrap_or_else(|_| unreachable!("BOOTED admits one init"))
        });
    }

    pub fn with<R>(f: impl FnOnce(&JsRuntime) -> R) -> R {
        RUNTIME.with(|r| f(r.get().expect("JsRuntime::init has not run on this thread")))
    }

    pub fn try_with<R>(f: impl FnOnce(&JsRuntime) -> R) -> Option<R> {
        RUNTIME.with(|r| r.get().map(f))
    }

    pub fn raw(&self) -> *mut c_void {
        self.ctx.raw()
    }

    pub fn ctx(&self) -> &Context {
        &self.ctx
    }

    pub fn eval(&self, src: &str, url: &str) -> Result<(), JsError> {
        match self.ctx.eval_named(src, url) {
            Ok(_) => Ok(()),
            Err(Error::Exception(e)) => Err(self.describe(e)),
            Err(e) => Err(JsError {
                header: format!("engine error: {e}"),
                stack: String::new(),
            }),
        }
    }

    pub fn register(&self, name: &str, arity: u32, f: *const c_void) {
        let v = unsafe { self.ctx.function_sysv(name, arity, f) }
            .unwrap_or_else(|e| panic!("registering host fn {name}: {e:?}"));
        self.ctx
            .global_set(name, v)
            .unwrap_or_else(|e| panic!("defining global {name}: {e:?}"));
    }

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

    pub fn drain_errors(&self) -> Vec<JsError> {
        ERRORS.with(|e| e.borrow_mut().drain(..).collect())
    }

    /// Installs the host content provider consulted by `fetch()` and image loads before the disk roots. The
    /// closure lives in a process global behind a trampoline (the raw ABI has no user-data pointer), hence
    /// `Send + Sync`. One provider per process; a second call is a no-op with an advisory.
    pub fn set_content_provider(
        &self,
        f: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static,
    ) {
        if PROVIDER.set(Box::new(f)).is_err() {
            eprintln!(
                "[sk-runtime] set_content_provider called twice — keeping the first provider"
            );
            return;
        }
        unsafe { ffi::redjs_content_provider_set(self.ctx.raw(), content_read as *const c_void) };
    }

    pub fn clear_content_provider(&self) {
        unsafe { ffi::redjs_content_provider_set(self.ctx.raw(), std::ptr::null()) };
    }

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

const _: ContentReadFn = content_read;

/// C-ABI trampoline for `redjs_content_provider_set`; a panic becomes a miss rather than unwinding through redjs.
unsafe extern "C" fn content_read(
    path_ptr: *const u8,
    path_len: usize,
    sink: ContentSinkFn,
    sink_ctx: *mut c_void,
) -> i32 {
    const MISS: i32 = 1;
    let Some(provider) = PROVIDER.get() else {
        return MISS;
    };
    if path_ptr.is_null() {
        return MISS;
    }
    let bytes = core::slice::from_raw_parts(path_ptr, path_len);
    let Ok(key) = std::str::from_utf8(bytes) else {
        return MISS;
    };
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider(key)));
    match found {
        Ok(Some(data)) => {
            sink(sink_ctx, data.as_ptr(), data.len());
            0
        }
        Ok(None) => MISS,
        Err(_) => {
            eprintln!("[sk-runtime] content provider panicked for {key:?} — treated as a miss");
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
        let header = if name.is_empty() {
            msg
        } else {
            format!("{name}: {msg}")
        };
        JsError { header, stack }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigator_reports_the_skathi_user_agent() {
        assert_eq!(SK_USER_AGENT, redjs::webapi::navigator::DEFAULT_USER_AGENT);
        unsafe { assert_eq!(ffi::redjs_init(std::ptr::null()), 0) };
        apply_user_agent();
        let ctx = Context::new();
        unsafe {
            assert_eq!(ffi::redjs_web_install(ctx.raw()), 0);
            let root: Vec<u16> = NO_DISK_ROOT.encode_utf16().collect();
            assert_eq!(
                ffi::redjs_webshim_install(
                    ctx.raw(),
                    root.as_ptr(),
                    root.len(),
                    root.as_ptr(),
                    root.len()
                ),
                0
            );
        }
        let read = |src: &str| ctx.to_rust_string(ctx.eval(src).unwrap()).unwrap();
        assert_eq!(read("navigator.userAgent"), SK_USER_AGENT);
        assert_eq!(
            read("navigator.appVersion"),
            redjs::webapi::navigator::app_version(SK_USER_AGENT)
        );
        assert_eq!(
            read("navigator.platform"),
            redjs::webapi::navigator::platform(SK_USER_AGENT)
        );
        #[cfg(target_os = "macos")]
        assert_eq!(read("navigator.platform"), "MacIntel");
        #[cfg(target_os = "windows")]
        assert_eq!(read("navigator.platform"), "Win32");
        assert_eq!(read("String(navigator instanceof Navigator)"), "true");
    }

    #[cfg(feature = "indexeddb")]
    #[test]
    fn indexeddb_answers_an_awaited_chain_within_a_few_frames() {
        unsafe { assert_eq!(ffi::redjs_init(std::ptr::null()), 0) };
        apply_pump_budget();
        assert_eq!(redjs::web::pump_budget_us(), SK_PUMP_DELIVERY_BUDGET_US);
        let ctx = Context::new();
        unsafe {
            assert_eq!(ffi::redjs_web_install(ctx.raw()), 0);
            let root: Vec<u16> = NO_DISK_ROOT.encode_utf16().collect();
            assert_eq!(
                ffi::redjs_webshim_install(
                    ctx.raw(),
                    root.as_ptr(),
                    root.len(),
                    root.as_ptr(),
                    root.len()
                ),
                0
            );
        }
        let read = |src: &str| ctx.to_rust_string(ctx.eval(src).unwrap()).unwrap();
        assert_eq!(read("typeof indexedDB + ',' + typeof IDBKeyRange + ',' + typeof navigator.storage.estimate"), "object,function,function");
        read(
            "globalThis.__r = '';
             const p = (q) => new Promise((res, rej) => { q.onsuccess = () => res(q.result); q.onerror = () => rej(q.error); });
             (async () => {
               const open = indexedDB.open('sk-runtime-chain', 1);
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
                ffi::redjs_pump(
                    ctx.raw(),
                    PUMP_TIMERS | PUMP_RAF | PUMP_MICROTASKS,
                    std::ptr::null_mut(),
                );
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
