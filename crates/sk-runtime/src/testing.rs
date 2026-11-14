//! The JS runtime for tests: one per process, on one thread. JSC boots once per process, and
//! `JsRuntime` is thread-affine, so a test that needs JS runs its body on one long-lived thread
//! ([`on_js_thread`]). Bodies share that thread's global object and thread-locals, so a test sets up
//! what it reads; a panic in a body fails only that test.

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use crate::JsRuntime;

type Job = Box<dyn FnOnce() + Send>;

static JS_THREAD: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();

pub fn on_js_thread<R, F>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let jobs = JS_THREAD.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("sk-runtime test JS thread".into())
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
