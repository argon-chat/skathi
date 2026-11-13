//! The file I/O pool: four worker threads on one queue. Every File System Access operation that
//! touches a user path runs here; the frame thread only validates and queues.
//!
//! [`in_flight`] counts jobs submitted and not finished. A job settles its token BEFORE the count
//! drops, so the pump source never sees "nothing queued, nothing in flight" while a result is on its
//! way. Dialogs and the shell are not counted: a Save As left open must not make every frame wait.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

use crate::Fail;

/// Enough to overlap a slow network share with local work, few enough that a burst of `getFile()`
/// is not a thread per file.
pub const WORKERS: usize = 4;

type Job = Box<dyn FnOnce() + Send>;

struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
}

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

pub fn in_flight() -> usize {
    IN_FLIGHT.load(Ordering::Acquire)
}

fn queue() -> &'static Queue {
    static Q: OnceLock<Queue> = OnceLock::new();
    static STARTED: OnceLock<usize> = OnceLock::new();
    let q = Q.get_or_init(|| Queue { jobs: Mutex::new(VecDeque::new()), ready: Condvar::new() });
    STARTED.get_or_init(|| {
        let mut started = 0;
        for i in 0..WORKERS {
            let spawned = std::thread::Builder::new().name(format!("vn-fs-{i}")).spawn(worker);
            if spawned.is_ok() {
                started += 1;
            }
        }
        started
    });
    q
}

fn worker() {
    let q = queue();
    loop {
        let job = {
            let mut jobs = q.jobs.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(j) = jobs.pop_front() {
                    break j;
                }
                jobs = q.ready.wait(jobs).unwrap_or_else(|p| p.into_inner());
            }
        };
        run(job);
    }
}

/// Runs one job, then drops the in-flight count (it has settled) and wakes the pump. [`submit`]
/// already turned a panic into a rejection.
fn run(job: Job) {
    job();
    IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    crate::wake();
}

/// Runs `op` on the pool and settles `token`; a panic settles `InvalidStateError` rather than
/// leaving the promise pending.
pub(crate) fn submit(token: u32, op: impl FnOnce() -> Result<crate::Done, Fail> + Send + 'static) {
    IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(op)).unwrap_or_else(|_| {
            Err(Fail::new("InvalidStateError", "the file operation failed inside the host (a panic on the I/O pool)"))
        });
        crate::settle_done(token, result);
    });
    let q = queue();
    {
        let mut jobs = q.jobs.lock().unwrap_or_else(|p| p.into_inner());
        jobs.push_back(job);
    }
    q.ready.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_settles_and_a_panic_becomes_a_rejection() {
        let ok = crate::next_token();
        let bad = crate::next_token();
        submit(ok, || Ok(crate::Done::json("{\"x\":1}")));
        submit(bad, || panic!("a pool job panics on purpose (test)"));
        let got = crate::tests::wait_for_settles(&[ok, bad]);
        assert_eq!(got[&ok], (true, "{\"x\":1}".to_string()));
        let (okb, payload) = &got[&bad];
        assert!(!okb);
        assert!(payload.contains("InvalidStateError"), "{payload}");
    }
}
