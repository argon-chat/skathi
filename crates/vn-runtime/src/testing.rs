//! The JS runtime for tests: one per process, on one thread.
//!
//! [`JsRuntime::init`] boots JSC and redjs's web layer, which exist once per
//! process. A second boot, from a second test, used to end the whole test
//! binary in a fast-fail (`0xC0000409`, STATUS_STACK_BUFFER_OVERRUN) with no
//! message: it applies JSC's options again, after the first VM froze them.
//! In a parallel run the abort was reported against whichever test happened
//! to be running. `init` now refuses a second call with a panic that names
//! this module; a test that needs JS runs its body here instead:
//!
//! ```ignore
//! #[test]
//! fn a_blob_url_resolves() {
//!     vn_runtime::testing::on_js_thread(|| {
//!         JsRuntime::with(|rt| rt.eval("1 + 1", "test").unwrap());
//!         // …the assertions, on the thread that owns the runtime.
//!     });
//! }
//! ```
//!
//! The runtime is thread-affine (`JsRuntime` lives in a thread-local), so the
//! body is not run on the test's own thread but on one long-lived thread,
//! created on first use, that booted the runtime. The bodies of all the tests
//! of one binary run there one at a time, in the order they ask, and share
//! everything that thread holds: the JS global object, and every thread-local
//! of the crates under test (vn-tree's tree among them). A test sets up what it
//! reads and does not count on a clean global.
//!
//! A panic in the body fails the test that ran it, with the body's payload.
//! The thread keeps serving the other tests.

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use crate::JsRuntime;

type Job = Box<dyn FnOnce() + Send>;

static JS_THREAD: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();

/// Runs `f` on this process's JS thread, where [`JsRuntime::init`] has run, and
/// returns what it returns. See the module header.
pub fn on_js_thread<R, F>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let jobs = JS_THREAD.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("vn-runtime test JS thread".into())
            .spawn(move || {
                JsRuntime::init();
                for job in rx {
                    job();
                }
            })
            .expect("could not start the JS test thread");
        Mutex::new(tx)
    });
    let (done, result) = mpsc::sync_channel(1);
    let job: Job = Box::new(move || {
        let _ = done.send(panic::catch_unwind(AssertUnwindSafe(f)));
    });
    jobs.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .send(job)
        .expect("the JS test thread has exited");
    match result.recv().expect("the JS test thread exited while running a test body") {
        Ok(value) => value,
        Err(payload) => panic::resume_unwind(payload),
    }
}
