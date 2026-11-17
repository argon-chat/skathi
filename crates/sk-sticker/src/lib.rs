//! sk-sticker — the engine behind `<sk-sticker>`: Lottie (`.tgs`) and VP9-alpha WebM stickers and
//! animated emoji, many on screen at once, drawn as cheaply as the frame allows.
//!
//! The element (sk-tree) asks for a source at a size; this crate draws it off the UI thread and
//! hands back straight-alpha BGRA frames for a `LiveTile`. One engine per source and one frame per
//! size are shared by every place that shows it, all on the source's clock; small loops are kept
//! compressed so their engine can be freed. [`core`] has the model and the budgets.
//!
//! ```ignore
//! let stickers = sk_sticker::global();
//! match stickers.status(src) {
//!     SourceStatus::NeedsBytes => { /* fetch, then */ stickers.provide(src, bytes) }
//!     SourceStatus::Failed(why) => { /* `error` event */ }
//!     _ => {}
//! }
//! stickers.demand(&key, visible);
//! if let Some(frame) = stickers.frame(&key) { /* write it if its seq is new */ }
//! ```
//!
//! [`video`] is the VP9 track (WebM demux, libvpx, I420+A to BGRA; its contract is that module's
//! docs); everything else is the core's.

pub mod cache;
pub mod clock;
pub mod core;
pub mod engine;
#[cfg(feature = "video")]
pub mod video;

use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub use crate::core::{Frame, SourceInfo, SourceStatus, Stats, TargetKey};
pub use crate::engine::Kind;

use crate::clock::Us;
use crate::core::Core;

pub struct Stickers {
    shared: Arc<Shared>,
}

struct Shared {
    core: Mutex<Core>,
    wake: Condvar,
    epoch: Instant,
}

impl Shared {
    fn now(&self) -> Us {
        self.epoch.elapsed().as_micros() as Us
    }

    fn lock(&self) -> MutexGuard<'_, Core> {
        // A panic mid-turn leaves the maps consistent (every mutation is a
        // single insert or field store), so carrying on is right.
        self.core
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

static GLOBAL: OnceLock<Stickers> = OnceLock::new();

pub fn global() -> &'static Stickers {
    GLOBAL.get_or_init(|| Stickers::start(engine::load))
}

/// The process engine's stats, or `None` when nothing has used it (asking does not start a worker).
pub fn stats() -> Option<Stats> {
    GLOBAL.get().map(Stickers::stats)
}

impl Stickers {
    pub fn start(loader: engine::Loader) -> Stickers {
        let shared = Arc::new(Shared {
            core: Mutex::new(Core::new(loader)),
            wake: Condvar::new(),
            epoch: Instant::now(),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("sk-sticker".into())
            .spawn(move || work(&worker))
            .expect("spawn the sk-sticker worker");
        Stickers { shared }
    }

    pub fn status(&self, src: &str) -> SourceStatus {
        let now = self.shared.now();
        self.shared.lock().status(src, now)
    }

    pub fn provide(&self, src: &str, bytes: Arc<[u8]>) {
        let now = self.shared.now();
        if self.shared.lock().provide(src, bytes, now) {
            self.shared.wake.notify_one();
        }
    }

    pub fn fail(&self, src: &str, why: &str) {
        let now = self.shared.now();
        self.shared.lock().fail(src, why, now);
    }

    pub fn demand(&self, key: &TargetKey, visible: bool) {
        let now = self.shared.now();
        if self.shared.lock().demand(key, visible, now) {
            self.shared.wake.notify_one();
        }
    }

    pub fn frame(&self, key: &TargetKey) -> Option<Frame> {
        let now = self.shared.now();
        let (frame, promoted) = self.shared.lock().frame(key, now);
        if promoted {
            self.shared.wake.notify_one();
        }
        frame
    }

    pub fn stats(&self) -> Stats {
        self.shared.lock().stats()
    }
}

/// The worker: plan under the lock, draw without it, deliver under it, and
/// sleep until woken or the next timer.
fn work(shared: &Shared) {
    loop {
        let mut core = shared.lock();
        let now = shared.now();
        let jobs = core.plan(now);
        if jobs.is_empty() {
            let wait = core.next_wake(now) - now;
            drop(shared.wake.wait_timeout(core, Duration::from_micros(wait)));
            continue;
        }
        drop(core);
        let done: Vec<_> = jobs.into_iter().map(crate::core::run).collect();
        let now = shared.now();
        shared.lock().deliver(done, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worker_draws_a_tgs_off_the_calling_thread() {
        let stickers = Stickers::start(engine::load);
        let src = "fixture://spinning-star.tgs";
        assert_eq!(stickers.status(src), SourceStatus::NeedsBytes);
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sk-lottie/tests/fixtures/spinning-star.tgs"
        ))
        .unwrap();
        stickers.provide(src, bytes.into());
        let key = TargetKey::new(Arc::from(src), 48, 48);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seqs = Vec::new();
        while seqs.len() < 3 {
            assert!(
                Instant::now() < deadline,
                "no frames: {:?}",
                stickers.stats()
            );
            stickers.demand(&key, true);
            if let Some(f) = stickers.frame(&key) {
                assert_eq!(f.bytes.len(), 48 * 48 * 4);
                if seqs.last() != Some(&f.seq) {
                    seqs.push(f.seq);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let stats = stickers.stats();
        assert!(matches!(
            stickers.status(src),
            SourceStatus::Ready(SourceInfo { frames: 120, .. })
        ));
        assert!(stats.drawn >= 3, "{stats:?}");
    }
}
