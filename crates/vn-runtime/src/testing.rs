//! The JS runtime for tests: one per process, on one thread.
//!
//! JSC and redjs boot once per process (a second boot fast-fails the test binary), and
//! `JsRuntime` is thread-affine, so a test that needs JS runs its body on one long-lived thread:
//!
//! ```ignore
//! vn_runtime::testing::on_js_thread(|| {
//!     JsRuntime::with(|rt| rt.eval("1 + 1", "test").unwrap());
//! });
//! ```
//!
//! Bodies run one at a time and share that thread's global object and thread-locals, so a test
//! sets up what it reads. A panic in a body fails only the test that ran it.

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use crate::JsRuntime;

type Job = Box<dyn FnOnce() + Send>;

static JS_THREAD: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();

/// Runs `f` on this process's JS thread, where [`JsRuntime::init`] has run.
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
