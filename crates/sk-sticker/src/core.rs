//! The registry and the scheduler, with no thread and no clock of its own: every call takes `now`,
//! and [`Core::plan`] / [`run`] / [`Core::deliver`] are one turn of the worker. [`crate::Stickers`]
//! puts it behind a lock with a worker thread; tests drive it by hand.
//!
//! * A **source** is one `src`: its bytes, [`SourceInfo`], its [`Engine`] while loaded, its [`Clock`].
//! * A **target** is a source at one size in device pixels ([`TargetKey`]), shared by every place
//!   showing it: one frame, one upload a window. The element (sk-tree) owns the `LiveTile`s and
//!   writes [`Frame`]s into them in its tick.
//! * Each tick the element says which targets it shows and whether any place is visible
//!   ([`Core::demand`]) and takes each target's due frame ([`Core::frame`]). The worker keeps one
//!   frame ahead of each visible target.
//!
//! # Budgets
//!
//! * Targets ≤ [`SMALL_PX`] play at most [`SMALL_FPS`] and their loops are cached ([`crate::cache`])
//!   under [`CACHE_BUDGET`]; a source whose every target replays from cache drops its engine.
//! * A target no element asked for in [`TARGET_IDLE`] is dropped (its loop cache stays).
//! * A source's engine is dropped [`SOURCE_IDLE`] after it last played; its bytes, [`SOURCE_IDLE`]
//!   after it was last asked for.
//! * At most [`MAX_LARGE_VIDEO`] video engines over [`LARGE_VIDEO_AREA`] are loaded; another waits
//!   for one that is not playing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::cache::{compress, LoopCache};
use crate::clock::{grid_ceil, Clock, Pace, Timeline, Us};
use crate::engine::{Engine, Kind, Loader};

/// Targets this size or smaller (both sides, device px) are "emoji": capped
/// at [`SMALL_FPS`] and loop-cached.
pub const SMALL_PX: u32 = 64;
pub const SMALL_FPS: f32 = 30.0;
pub const CACHE_BUDGET: usize = 24 << 20;
pub const TARGET_IDLE: Us = 3_000_000;
pub const SOURCE_IDLE: Us = 15_000_000;
pub const MAX_LARGE_VIDEO: usize = 4;
/// A video larger than this (w × h) is a "512 px" decoder: ~5 MB with alpha.
pub const LARGE_VIDEO_AREA: u64 = 256 * 256;
/// Loops being filled at once; others play uncached until one completes.
pub const MAX_FILLING: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TargetKey {
    pub src: Arc<str>,
    pub width: u32,
    pub height: u32,
    /// A tinted video: decode the alpha stream only (B = G = R = 255). Video only; a Lottie is tinted at paint time.
    pub alpha_only: bool,
    pub max_fps: Option<u16>,
    pub still: bool,
    /// Plays one loop on its own clock and holds the last frame; the value
    /// (the element's node id) keeps two such copies apart.
    pub once: Option<u32>,
}

impl TargetKey {
    pub fn new(src: Arc<str>, width: u32, height: u32) -> TargetKey {
        TargetKey { src, width, height, alpha_only: false, max_fps: None, still: false, once: None }
    }

    pub fn is_small(&self) -> bool {
        self.width <= SMALL_PX && self.height <= SMALL_PX
    }

    fn cap(&self) -> Option<f32> {
        let cap = self.max_fps.map(f32::from);
        if self.is_small() {
            Some(cap.map_or(SMALL_FPS, |c| c.min(SMALL_FPS)))
        } else {
            cap
        }
    }

    fn cacheable(&self) -> bool {
        self.is_small() && !self.still && self.once.is_none()
    }

    fn shares_clock(&self) -> bool {
        !self.still && self.once.is_none()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub fps: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SourceStatus {
    NeedsBytes,
    Loading,
    Ready(SourceInfo),
    Failed(Arc<str>),
}

#[derive(Clone, Debug)]
pub struct Frame {
    /// Unique per frame drawn: a tile writes when it changes.
    pub seq: u64,
    pub slot: u32,
    pub width: u32,
    pub height: u32,
    pub bytes: Arc<Vec<u8>>,
    /// A `once` target has played its loop: this is its last frame for good.
    pub ended: bool,
    due: Us,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub sources: usize,
    pub file_bytes: usize,
    pub engines: usize,
    pub engine_bytes: usize,
    pub targets: usize,
    pub visible_targets: usize,
    pub frame_bytes: usize,
    pub loops: usize,
    pub loops_complete: usize,
    pub cache_bytes: usize,
    pub cache_budget: usize,
    pub drawn: u64,
    pub from_cache: u64,
    pub loads: u64,
    pub failures: u64,
}

#[derive(Debug, PartialEq)]
enum State {
    Pending,
    Loading,
    Ready,
    Failed(Arc<str>),
}

struct Source {
    state: State,
    bytes: Option<Arc<[u8]>>,
    info: Option<SourceInfo>,
    timeline: Option<Timeline>,
    engine: Option<Box<dyn Engine>>,
    busy: bool,
    clock: Option<Clock>,
    last_demand: Us,
    last_active: Us,
}

impl Source {
    fn new(now: Us) -> Source {
        Source {
            state: State::Pending,
            bytes: None,
            info: None,
            timeline: None,
            engine: None,
            busy: false,
            clock: None,
            last_demand: now,
            last_active: now,
        }
    }

    fn is_large_video(&self) -> bool {
        self.info
            .as_ref()
            .is_some_and(|i| i.kind == Kind::Video && i.width as u64 * i.height as u64 > LARGE_VIDEO_AREA)
    }
}

struct Target {
    visible: bool,
    last_demand: Us,
    current: Option<Frame>,
    ready: Option<Frame>,
    drawing: bool,
    clock: Option<Clock>,
    /// Its loop did not fit the budget: drawn live.
    no_cache: bool,
}

impl Target {
    fn has_frame(&self) -> bool {
        self.current.is_some() || self.ready.is_some()
    }
}

pub struct Req {
    key: TargetKey,
    slot: u32,
    slots: u32,
    frame: u32,
    due: Us,
    cache: bool,
}

pub enum Job {
    Load { src: Arc<str>, bytes: Arc<[u8]>, loader: Loader },
    Draw { src: Arc<str>, engine: Box<dyn Engine>, reqs: Vec<Req> },
}

pub type Drawn = (Req, Result<Vec<u8>, String>, Option<Box<[u8]>>);

pub enum Done {
    Loaded { src: Arc<str>, result: Result<Box<dyn Engine>, String> },
    Drawn { src: Arc<str>, engine: Box<dyn Engine>, frames: Vec<Drawn> },
}

/// Does a job: loads, or draws (in frame order, which a video decoder needs) and compresses.
pub fn run(job: Job) -> Done {
    match job {
        Job::Load { src, bytes, loader } => Done::Loaded { src, result: loader(&bytes) },
        Job::Draw { src, mut engine, mut reqs } => {
            reqs.sort_by_key(|r| r.frame);
            let frames = reqs
                .into_iter()
                .map(|req| {
                    let drawn = engine.draw(req.frame, req.key.alpha_only, req.key.width, req.key.height);
                    let packed = match (&drawn, req.cache) {
                        (Ok(px), true) => Some(compress(px)),
                        _ => None,
                    };
                    (req, drawn, packed)
                })
                .collect();
            Done::Drawn { src, engine, frames }
        }
    }
}

pub struct Core {
    loader: Loader,
    sources: HashMap<Arc<str>, Source>,
    targets: HashMap<TargetKey, Target>,
    caches: HashMap<TargetKey, LoopCache>,
    cache_bytes: usize,
    cache_budget: usize,
    seq: u64,
    drawn: u64,
    from_cache: u64,
    loads: u64,
    failures: u64,
}

impl Core {
    pub fn new(loader: Loader) -> Core {
        Core::with_budget(loader, CACHE_BUDGET)
    }

    pub fn with_budget(loader: Loader, cache_budget: usize) -> Core {
        Core {
            loader,
            sources: HashMap::new(),
            targets: HashMap::new(),
            caches: HashMap::new(),
            cache_bytes: 0,
            cache_budget,
            seq: 0,
            drawn: 0,
            from_cache: 0,
            loads: 0,
            failures: 0,
        }
    }

    fn source(&mut self, src: &str, now: Us) -> &mut Source {
        if !self.sources.contains_key(src) {
            self.sources.insert(Arc::from(src), Source::new(now));
        }
        self.sources.get_mut(src).expect("just inserted")
    }

    pub fn status(&mut self, src: &str, now: Us) -> SourceStatus {
        let s = self.source(src, now);
        s.last_demand = now;
        match &s.state {
            State::Pending => SourceStatus::NeedsBytes,
            State::Loading => SourceStatus::Loading,
            State::Ready => SourceStatus::Ready(s.info.clone().expect("a ready source has its info")),
            State::Failed(why) => SourceStatus::Failed(why.clone()),
        }
    }

    pub fn provide(&mut self, src: &str, bytes: Arc<[u8]>, now: Us) -> bool {
        let s = self.source(src, now);
        if s.state != State::Pending {
            return false;
        }
        s.bytes = Some(bytes);
        s.state = if s.info.is_some() { State::Ready } else { State::Loading };
        true
    }

    pub fn fail(&mut self, src: &str, why: &str, now: Us) {
        let s = self.source(src, now);
        s.state = State::Failed(Arc::from(why));
        s.bytes = None;
        self.failures += 1;
    }

    pub fn demand(&mut self, key: &TargetKey, visible: bool, now: Us) -> bool {
        let s = self.source(&key.src, now);
        s.last_demand = now;
        if visible && key.shares_clock() {
            s.last_active = now;
        }
        let mut wake = false;
        let t = self.targets.entry(key.clone()).or_insert_with(|| {
            wake = true;
            Target {
                visible,
                last_demand: now,
                current: None,
                ready: None,
                drawing: false,
                clock: None,
                no_cache: false,
            }
        });
        wake |= visible && !t.visible;
        t.visible = visible;
        t.last_demand = now;
        if key.once.is_some() && visible && t.clock.is_none() {
            t.clock = Some(Clock::new(now));
        }
        wake
    }

    /// `key`'s frame at `now`, promoting the next one when it is due (the bool says one was).
    pub fn frame(&mut self, key: &TargetKey, now: Us) -> (Option<Frame>, bool) {
        let Some(t) = self.targets.get_mut(key) else { return (None, false) };
        let due = t.ready.as_ref().is_some_and(|r| r.due <= now);
        // A hidden target keeps the frame it has: the one drawn ahead belongs to a clock that has
        // stopped and would put the target ahead of it when it resumes.
        let promoted = due && (t.visible || t.current.is_none());
        if promoted {
            t.current = t.ready.take();
        } else if due {
            t.ready = None;
        }
        let mut frame = t.current.clone();
        if let (Some(f), Some(clock), Some(timeline)) =
            (frame.as_mut(), t.clock.as_ref(), self.sources.get(&key.src).and_then(|s| s.timeline.as_ref()))
        {
            let pace = Pace::new(timeline, key.cap());
            f.ended = clock.elapsed(now) >= pace.duration() && f.slot + 1 == pace.slots();
        }
        (frame, promoted)
    }

    /// One worker turn's work: timers, clocks, loads, and the next frame of every target that needs one.
    pub fn plan(&mut self, now: Us) -> Vec<Job> {
        self.maintain(now);
        let active = self.run_clocks(now);
        let mut jobs = Vec::new();

        let wanted: HashSet<Arc<str>> = self
            .targets
            .iter()
            .filter(|(k, t)| self.wants_engine_now(k, t))
            .map(|(k, _)| k.src.clone())
            .collect();
        let srcs: Vec<Arc<str>> = self.sources.keys().cloned().collect();
        for src in srcs {
            let s = &self.sources[&src];
            if s.busy || s.engine.is_some() {
                continue;
            }
            let load = match s.state {
                State::Loading => true,
                State::Ready => wanted.contains(&src),
                _ => false,
            };
            if !load {
                continue;
            }
            if s.bytes.is_none() {
                self.sources.get_mut(&src).expect("listed").state = State::Pending;
                continue;
            }
            if s.is_large_video() && !self.make_room_for_large_video(&active) {
                continue;
            }
            let s = self.sources.get_mut(&src).expect("listed");
            s.busy = true;
            jobs.push(Job::Load { src: src.clone(), bytes: s.bytes.clone().expect("checked"), loader: self.loader });
        }

        let filling = self.caches.values().filter(|c| !c.is_complete()).count();
        let mut new_fills = 0usize;
        let mut draws: HashMap<Arc<str>, Vec<Req>> = HashMap::new();
        let keys: Vec<TargetKey> = self.targets.keys().cloned().collect();
        for key in keys {
            let Some(s) = self.sources.get(&key.src) else { continue };
            if s.state != State::Ready {
                continue;
            }
            let Some(timeline) = s.timeline.clone() else { continue };
            let t = &self.targets[&key];
            if t.drawing {
                continue;
            }
            let Some((slot, due)) = next_slot(&key, t, s.clock.as_ref(), &timeline, now) else { continue };
            let pace = Pace::new(&timeline, key.cap());
            let len = key.width as usize * key.height as usize * 4;
            let cacheable = key.cacheable() && !t.no_cache;
            if cacheable {
                if let Some(px) = self.caches.get_mut(&key).and_then(|c| c.get(slot, len, now)) {
                    self.seq += 1;
                    self.from_cache += 1;
                    let frame = Frame {
                        seq: self.seq,
                        slot,
                        width: key.width,
                        height: key.height,
                        bytes: Arc::new(px),
                        ended: false,
                        due,
                    };
                    self.targets.get_mut(&key).expect("listed").ready = Some(frame);
                    continue;
                }
            }
            let s = &self.sources[&key.src];
            if s.busy || s.engine.is_none() {
                continue;
            }
            let fill = cacheable
                && (self.caches.contains_key(&key) || {
                    let room = filling + new_fills < MAX_FILLING;
                    new_fills += usize::from(room);
                    room
                });
            let req = Req { key: key.clone(), slot, slots: pace.slots(), frame: pace.frame(&timeline, slot), due, cache: fill };
            self.targets.get_mut(&key).expect("listed").drawing = true;
            draws.entry(key.src.clone()).or_default().push(req);
        }
        for (src, reqs) in draws {
            let s = self.sources.get_mut(&src).expect("drawn from");
            let engine = s.engine.take().expect("checked");
            s.busy = true;
            jobs.push(Job::Draw { src, engine, reqs });
        }
        jobs
    }

    pub fn deliver(&mut self, done: Vec<Done>, now: Us) {
        for d in done {
            match d {
                Done::Loaded { src, result } => {
                    let Some(s) = self.sources.get_mut(&src) else { continue };
                    s.busy = false;
                    match result {
                        Ok(engine) => {
                            self.loads += 1;
                            let (width, height) = engine.size();
                            let timeline = engine.timeline();
                            s.info = Some(SourceInfo {
                                kind: engine.kind(),
                                width,
                                height,
                                frames: timeline.frames(),
                                fps: timeline.fps(),
                            });
                            s.timeline = Some(timeline);
                            s.engine = Some(engine);
                            s.state = State::Ready;
                            s.last_active = now;
                            if s.is_large_video() {
                                self.trim_large_videos(&src);
                            }
                        }
                        Err(why) => {
                            self.failures += 1;
                            s.state = State::Failed(Arc::from(why.as_str()));
                            s.bytes = None;
                        }
                    }
                }
                Done::Drawn { src, engine, frames } => {
                    let mut failed = None;
                    for (req, drawn, packed) in frames {
                        let Some(t) = self.targets.get_mut(&req.key) else { continue };
                        t.drawing = false;
                        match drawn {
                            Ok(px) => {
                                self.drawn += 1;
                                self.seq += 1;
                                t.ready = Some(Frame {
                                    seq: self.seq,
                                    slot: req.slot,
                                    width: req.key.width,
                                    height: req.key.height,
                                    bytes: Arc::new(px),
                                    ended: false,
                                    due: req.due,
                                });
                                if let Some(packed) = packed {
                                    self.cache_put(&req.key, req.slot, req.slots, packed, now);
                                }
                            }
                            Err(why) => failed = Some(why),
                        }
                    }
                    let keep = self.needs_engine(&src);
                    let Some(s) = self.sources.get_mut(&src) else { continue };
                    s.busy = false;
                    if let Some(why) = failed {
                        self.failures += 1;
                        s.state = State::Failed(Arc::from(why.as_str()));
                        s.bytes = None;
                    } else if keep {
                        s.engine = Some(engine);
                    }
                }
            }
        }
    }

    pub fn next_wake(&self, now: Us) -> Us {
        let mut wake = now + 1_000_000;
        for t in self.targets.values() {
            wake = wake.min(t.last_demand + TARGET_IDLE + 1);
        }
        for s in self.sources.values() {
            if s.engine.is_some() {
                wake = wake.min(s.last_active + SOURCE_IDLE + 1);
            }
            wake = wake.min(s.last_demand + SOURCE_IDLE + 1);
        }
        wake.max(now + 1_000)
    }

    pub fn stats(&self) -> Stats {
        let frame_bytes = |f: &Option<Frame>| f.as_ref().map_or(0, |f| f.bytes.len());
        Stats {
            sources: self.sources.len(),
            file_bytes: self.sources.values().filter_map(|s| s.bytes.as_ref()).map(|b| b.len()).sum(),
            engines: self.sources.values().filter(|s| s.engine.is_some() || (s.busy && s.state == State::Ready)).count(),
            engine_bytes: self.sources.values().filter_map(|s| s.engine.as_ref()).map(|e| e.estimated_bytes()).sum(),
            targets: self.targets.len(),
            visible_targets: self.targets.values().filter(|t| t.visible).count(),
            frame_bytes: self.targets.values().map(|t| frame_bytes(&t.current) + frame_bytes(&t.ready)).sum(),
            loops: self.caches.len(),
            loops_complete: self.caches.values().filter(|c| c.is_complete()).count(),
            cache_bytes: self.cache_bytes,
            cache_budget: self.cache_budget,
            drawn: self.drawn,
            from_cache: self.from_cache,
            loads: self.loads,
            failures: self.failures,
        }
    }

    fn maintain(&mut self, now: Us) {
        self.targets.retain(|_, t| t.drawing || now.saturating_sub(t.last_demand) <= TARGET_IDLE);
        let cached: HashSet<Arc<str>> = self.caches.keys().map(|k| k.src.clone()).collect();
        self.sources.retain(|src, s| {
            if s.busy {
                return true;
            }
            if s.engine.is_some() && now.saturating_sub(s.last_active) > SOURCE_IDLE {
                s.engine = None;
            }
            if now.saturating_sub(s.last_demand) <= SOURCE_IDLE {
                return true;
            }
            s.engine = None;
            s.bytes = None;
            if s.state == State::Ready && cached.contains(src) {
                return true; // its loops replay without it; it refetches for anything else
            }
            false
        });
    }

    fn run_clocks(&mut self, now: Us) -> HashSet<Arc<str>> {
        let active = self.playing();
        let mut resumed = HashSet::new();
        for (src, s) in &mut self.sources {
            match (&mut s.clock, active.contains(src)) {
                (None, true) => s.clock = Some(Clock::new(now)),
                (Some(clock), true) if !clock.is_running() => {
                    clock.resume(now);
                    resumed.insert(src.clone());
                }
                (Some(clock), false) => clock.pause(now),
                _ => {}
            }
        }
        // Frames drawn ahead before the stop are due by the old origin.
        for (k, t) in &mut self.targets {
            if resumed.contains(&k.src) && k.shares_clock() {
                t.ready = None;
            }
        }
        active
    }

    fn target_needs_engine(&self, key: &TargetKey, t: &Target) -> bool {
        if key.still {
            return !t.has_frame();
        }
        !(key.cacheable() && !t.no_cache && self.caches.get(key).is_some_and(LoopCache::is_complete))
    }

    fn wants_engine_now(&self, key: &TargetKey, t: &Target) -> bool {
        self.target_needs_engine(key, t) && (t.visible || !t.has_frame())
    }

    fn needs_engine(&self, src: &Arc<str>) -> bool {
        self.targets.iter().any(|(k, t)| &k.src == src && self.target_needs_engine(k, t))
    }

    /// Below [`MAX_LARGE_VIDEO`] large video engines, unloading one that is not playing if need be.
    fn make_room_for_large_video(&mut self, active: &HashSet<Arc<str>>) -> bool {
        let loaded = self.sources.values().filter(|s| s.is_large_video() && (s.engine.is_some() || s.busy)).count();
        if loaded < MAX_LARGE_VIDEO {
            return true;
        }
        let victim = self
            .sources
            .iter()
            .filter(|(src, s)| s.is_large_video() && s.engine.is_some() && !s.busy && !active.contains(*src))
            .min_by_key(|(_, s)| s.last_active)
            .map(|(src, _)| src.clone());
        match victim {
            Some(src) => {
                self.sources.get_mut(&src).expect("found").engine = None;
                true
            }
            None => false,
        }
    }

    /// Back to [`MAX_LARGE_VIDEO`] large video engines after `loaded` came in over it: unloads the
    /// least recently played that is not playing (maybe `loaded`). Over only when all play.
    fn trim_large_videos(&mut self, loaded: &Arc<str>) {
        let active = self.playing();
        loop {
            let count = self.sources.values().filter(|s| s.is_large_video() && (s.engine.is_some() || s.busy)).count();
            if count <= MAX_LARGE_VIDEO {
                return;
            }
            let victim = self
                .sources
                .iter()
                .filter(|(src, s)| s.is_large_video() && s.engine.is_some() && !s.busy && !active.contains(*src))
                .min_by_key(|(src, s)| (*src == loaded, s.last_active))
                .map(|(src, _)| src.clone());
            match victim {
                Some(src) => self.sources.get_mut(&src).expect("found").engine = None,
                None => return,
            }
        }
    }

    fn playing(&self) -> HashSet<Arc<str>> {
        self.targets.iter().filter(|(k, t)| t.visible && k.shares_clock()).map(|(k, _)| k.src.clone()).collect()
    }

    /// Keeps a compressed frame of `key`'s loop, evicting the least recently used loops of targets
    /// not shown to stay in budget; when nothing can go, the target is drawn live.
    fn cache_put(&mut self, key: &TargetKey, slot: u32, slots: u32, packed: Box<[u8]>, now: Us) {
        let add = packed.len();
        while self.cache_bytes + add > self.cache_budget {
            let victim = self
                .caches
                .iter()
                .filter(|(k, _)| *k != key && !self.targets.get(*k).is_some_and(|t| t.visible))
                .min_by_key(|(_, c)| c.last_used)
                .map(|(k, _)| k.clone());
            let gone = match victim {
                Some(k) => k,
                None => {
                    if let Some(c) = self.caches.remove(key) {
                        self.cache_bytes -= c.bytes();
                    }
                    if let Some(t) = self.targets.get_mut(key) {
                        t.no_cache = true;
                    }
                    return;
                }
            };
            let c = self.caches.remove(&gone).expect("found");
            self.cache_bytes -= c.bytes();
        }
        let c = self.caches.entry(key.clone()).or_insert_with(|| LoopCache::new(slots, now));
        self.cache_bytes += c.put(slot, packed, now);
    }
}

/// The slot `t` should hold next and when it is due, or `None` when it has it. A target with no
/// frame gets the one playing now; a hidden target, or one whose clock is stopped, keeps its frame;
/// a shown one behind its clock catches up, one on time gets the next slot, due at its start on the
/// present grid; a `once` target stops at its last slot.
fn next_slot(key: &TargetKey, t: &Target, shared: Option<&Clock>, timeline: &Timeline, now: Us) -> Option<(u32, Us)> {
    if t.ready.is_some() {
        return None;
    }
    if key.still {
        return (!t.has_frame()).then_some((0, now));
    }
    let pace = Pace::new(timeline, key.cap());
    let clock = if key.once.is_some() { t.clock.as_ref() } else { shared };
    let Some(clock) = clock else {
        return t.current.is_none().then_some((0, now));
    };
    let mut elapsed = clock.elapsed(now);
    if key.once.is_some() {
        elapsed = elapsed.min(pace.duration() - 1);
    }
    let (slot, next) = pace.at(elapsed);
    match &t.current {
        None => Some((slot, now)),
        Some(c) if c.slot != slot => t.visible.then_some((slot, now)),
        Some(_) if !t.visible || !clock.is_running() => None,
        Some(_) if key.once.is_some() && next >= pace.duration() => None,
        Some(_) => Some((pace.at(next).0, grid_ceil(clock.when(next)))),
    }
}

#[cfg(test)]
mod tests;
