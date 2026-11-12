//! `<lottie>` — the element layer over the `vn-lottie` crate.
//!
//! This module owns everything that outlives a frame: the per-node side map,
//! the `src` → bytes load, the prop→transport sync, the once-per-frame tick,
//! the rasters and the event dispatch. The gpui element itself is built in
//! [`crate::render`] (`build_lottie`), which is where the cascade, the focus
//! handles and the listener wiring already live.
//!
//! `crates/vn-lottie/CONTRACT.md` is the spec this implements; §"Element-layer
//! decisions" there records where the element layer deliberately overrides the
//! crate.
//!
//! # The three owner-level decisions, in one place
//!
//! 1. **`autoplay` and `loop` default to `false`.** The crate defaults both to
//!    lottie-web's `loadAnimation()` values (`true`/`true`); `<lottie>` is
//!    markup, and the markup-facing precedent is the `<lottie-player>` web
//!    component, which defaults both to `false`. Overridden here rather than in
//!    the crate so a Rust embedder driving `LottieState` directly still gets
//!    lottie-web's semantics.
//! 2. **`loopComplete` never fires on the boundary that completes playback.**
//!    lottie-web fires `complete` alone there. The crate's `Advance` reports
//!    both, so the last `LoopComplete` of a completing tick is filtered out at
//!    the dispatch site (see [`dispatch_events`]).
//! 3. **`:loop="n"` maps to `LoopMode::Count(n + 1)`.** lottie-web's numeric
//!    `loop` counts *repeats* — `loop: 2` plays three times — while the crate's
//!    `Count(n)` plays exactly `n` times. The `+ 1` is what makes `:loop="2"`
//!    behave like the browser; combined with (2) the event stream is then
//!    lottie-web's exactly: `loopComplete` ×n, then `complete`.
//!
//! # Off the UI thread (CONTRACT.md §8.10)
//!
//! The UI thread keeps only the transport ([`vn_lottie::Transport`]: the
//! playhead, the props, the events), which is arithmetic. Everything that
//! costs is on the `vn-lottie` worker thread:
//!
//! * **The load.** A file is read there, an http(s) `src` is fetched on gpui's
//!   HTTP client, and the parse and the engine's load run there. The node is an
//!   empty box until the load lands, as an `<img>` is until it decodes; the
//!   transport starts then, with the props it has at that moment.
//! * **The raster.** Each tick asks the worker for the frame at the playhead,
//!   at the box's size in device pixels, if the node can be seen. The next
//!   tick writes the drawn frame into the node's [`LiveTile`]: in place, a
//!   frame costs a present and no build. So the picture trails the transport
//!   by at most one vsync while the worker keeps up, and skips to the newest
//!   frame when it does not. Events follow the transport, as before.
//! * The pixels are straight alpha ([`vn_lottie::Animation::render_at`]), which
//!   is what gpui blends.
//!
//! A tile's first frame, a new size and a tile that moved in the atlas need a
//! draw, and touch the node, so a cache boundary over it renders again instead
//! of replaying a sprite that points at the old place. That is why `<lottie>`
//! is not one of `crate::cache`'s exclusions.
//!
//! # Detached nodes keep their clock running
//!
//! A `<KeepAlive>`-cached page is moved into a detached container, not
//! unmounted, so its nodes stay allocated and their entries stay in this map.
//! They keep advancing while off-screen — which is what a browser does with a
//! `<lottie-player>` in a detached subtree (its rAF loop is never cancelled) —
//! so coming back shows the animation where it *would* be, not where it was
//! left. Nothing is drawn meanwhile (a node no one can see asks for no raster),
//! so the cost is the transport arithmetic.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use gpui::{App, RenderImage, Window};
use vn_lottie::{Advance, Animation, Direction, LoopMode, LottieEvent, LottieMeta, PlayMode, Transport};

use crate::live::{self, LiveTile, Shown};
use crate::tree::{self, Tree};
use crate::windows::{self, WinKey};
use crate::EventPayload;

/// One `<lottie>` node's persistent state.
struct Entry {
    /// The raw `src` string the entry was built from — the (re)load key.
    src: String,
    /// `None` until the load lands, and for good when it failed. A broken `src`
    /// costs one advisory, not one per frame.
    transport: Option<Transport>,
    /// Last `autoplay` value seen, so play/pause is edge-triggered off the
    /// *prop* (a `v-bind` that never changes must not restart a paused
    /// animation every frame).
    autoplay: bool,
    /// **The window that built this node last** (docs/MULTIWINDOW.md §5.1).
    /// Its tile lives in that window's atlas; a node that moves to another
    /// window gets a new tile there.
    window: WinKey,
    /// The box the build laid the animation into: logical px, and the
    /// window's scale. What the raster is drawn for.
    boxed: Option<((f32, f32), f32)>,
    /// Shared with the worker: the animation, the load's outcome, the frame
    /// asked for and the frame drawn.
    job: Arc<Job>,
    /// The last raster asked for: `(frame bits, width, height)`.
    asked: Option<(u32, u32, u32)>,
    tile: Option<Tile>,
}

impl Entry {
    fn new(src: String, autoplay: bool, window: WinKey) -> Self {
        Self {
            src,
            transport: None,
            autoplay,
            window,
            boxed: None,
            job: Arc::new(Job::default()),
            asked: None,
            tile: None,
        }
    }

    /// The raster the box wants now: `(frame bits, width, height)`, and the frame.
    fn want(&self) -> Option<((u32, u32, u32), f32)> {
        let transport = self.transport.as_ref()?;
        let (size, scale) = self.boxed?;
        let (w, h) = vn_lottie::raster_size_for(transport.meta(), size, scale)?;
        let frame = transport.frame();
        Some(((frame.to_bits(), w, h), frame))
    }
}

/// The node's picture: a tile in `live`'s window, holding the frame `shows`.
struct Tile {
    live: LiveTile,
    shows: (u32, u32, u32),
}

/// A node's half of the worker's state. Locked only to hand things over: the
/// worker takes the animation out to draw.
#[derive(Default)]
struct Job {
    slot: Mutex<Slot>,
}

#[derive(Default)]
struct Slot {
    /// While the worker is not drawing with it.
    animation: Option<Animation>,
    /// What the load came to, until the tick takes it: the metadata, or the
    /// compat channel's key prefix and the advisory.
    loaded: Option<Result<LottieMeta, (&'static str, String)>>,
    /// Non-fatal notes from a load that worked, or the unsupported features of
    /// one that did not, for the compat channel.
    advisories: Vec<String>,
    /// The newest frame asked for and not started: `(frame, width, height)`.
    ask: Option<(f32, u32, u32)>,
    /// A draw is queued for this node.
    queued: bool,
    /// The newest frame drawn and not yet written: `(frame bits, w, h, bytes)`.
    drawn: Option<((u32, u32, u32), Vec<u8>)>,
    /// The node is gone: whatever is left is dropped.
    gone: bool,
}

impl Job {
    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(|p| p.into_inner())
    }
}

thread_local! {
    static LOTTIE_STATES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
    /// Wall clock for the tick. `None` until the first frame — the pump's own
    /// cadence is what defines `dt`, exactly as it does for CSS transitions.
    static LAST_TICK: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Live `<lottie>` elements (devtools `mem`).
pub(crate) fn count() -> usize {
    LOTTIE_STATES.with(|m| m.borrow().len())
}

/// A tick longer than this is treated as a stall (debugger, minimised window,
/// a long HMR patch) and clamped, so an animation does not teleport. Browsers
/// do the same by simply not running rAF while hidden.
const MAX_DT: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------------------
// the worker
// ---------------------------------------------------------------------------

/// Loads, fetches and draws not yet handed back. While any is out the pump
/// keeps ticking ([`any_playing`]), so the tick that takes the result comes.
static PENDING: AtomicUsize = AtomicUsize::new(0);

enum Work {
    /// Bytes in memory, or a file to read.
    Load { job: Arc<Job>, src: String, from: From },
    Draw(Arc<Job>),
}

enum From {
    Bytes(Vec<u8>),
    File(PathBuf),
}

struct Queue {
    work: Mutex<VecDeque<Work>>,
    ready: Condvar,
}

/// One thread: ThorVG serialises its calls on its engine lock anyway
/// (`vn_lottie::backend::thorvg`), so a second one would only wait.
fn queue() -> &'static Queue {
    static QUEUE: OnceLock<Queue> = OnceLock::new();
    QUEUE.get_or_init(|| {
        std::thread::Builder::new()
            .name("vn-lottie".into())
            .spawn(worker)
            .expect("spawn the vn-lottie thread");
        Queue { work: Mutex::new(VecDeque::new()), ready: Condvar::new() }
    })
}

fn submit(work: Work) {
    PENDING.fetch_add(1, Ordering::SeqCst);
    let q = queue();
    q.work.lock().unwrap_or_else(|p| p.into_inner()).push_back(work);
    q.ready.notify_one();
}

fn worker() {
    let q = queue();
    loop {
        let work = {
            let mut w = q.work.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(work) = w.pop_front() {
                    break work;
                }
                w = q.ready.wait(w).unwrap_or_else(|p| p.into_inner());
            }
        };
        match work {
            Work::Load { job, src, from } => load_on_worker(&job, &src, from),
            Work::Draw(job) => draw_on_worker(&job),
        }
        PENDING.fetch_sub(1, Ordering::SeqCst);
    }
}

fn load_on_worker(job: &Job, src: &str, from: From) {
    if job.slot().gone {
        return;
    }
    let bytes = match from {
        From::Bytes(bytes) => Ok(bytes),
        From::File(path) => crate::img::read_file("lottie", src.trim(), &path),
    };
    let (loaded, animation, advisories) = match bytes {
        Err(msg) => (Err(("lottie-src", msg)), None, Vec::new()),
        Ok(bytes) => match Animation::load(&bytes) {
            Ok(animation) => {
                let advisories = animation.advisories().to_vec();
                (Ok(animation.meta().clone()), Some(animation), advisories)
            }
            Err(error) => (
                Err(("lottie-load", format!("<lottie src=\"{src}\"> could not be loaded — {error}"))),
                None,
                error.advisories().to_vec(),
            ),
        },
    };
    let mut slot = job.slot();
    if !slot.gone {
        slot.animation = animation;
        slot.loaded = Some(loaded);
        slot.advisories = advisories;
    }
}

fn draw_on_worker(job: &Job) {
    let (ask, mut animation) = {
        let mut slot = job.slot();
        slot.queued = false;
        if slot.gone {
            return;
        }
        match (slot.ask.take(), slot.animation.take()) {
            (Some(ask), Some(animation)) => (ask, animation),
            (_, animation) => {
                slot.animation = animation;
                return;
            }
        }
    };
    let (frame, w, h) = ask;
    let bytes = animation.render_at(frame, w, h).map(<[u8]>::to_vec);
    let mut slot = job.slot();
    if slot.gone {
        return;
    }
    slot.animation = Some(animation);
    if let Some(bytes) = bytes {
        slot.drawn = Some(((frame.to_bits(), w, h), bytes));
    }
}

/// Asks the worker for `frame` at `w × h`; a newer ask replaces one not
/// started yet.
fn ask(job: &Arc<Job>, frame: f32, w: u32, h: u32) {
    let mut slot = job.slot();
    slot.ask = Some((frame, w, h));
    if !slot.queued {
        slot.queued = true;
        drop(slot);
        submit(Work::Draw(job.clone()));
    }
}

/// Starts loading `src` for `job`: a file is read on the worker, an http(s)
/// URL fetched on gpui's HTTP client, anything else (`data:`, `blob:`, the
/// package) is in memory already. `Err` is a `src` that cannot be resolved,
/// with what `<img>` would have said, for the caller to report now.
fn start_load(job: &Arc<Job>, src: &str, cx: &mut App) -> Result<(), String> {
    let trimmed = src.trim();
    let scheme = trimmed.split_once(':').map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
    if matches!(scheme.as_str(), "http" | "https") {
        PENDING.fetch_add(1, Ordering::SeqCst);
        let client = cx.http_client();
        let (job, src, url) = (job.clone(), src.to_string(), trimmed.to_string());
        cx.background_executor()
            .spawn(async move {
                match crate::fonts::http_bytes(client, &url).await {
                    Ok(bytes) => submit(Work::Load { job, src, from: From::Bytes(bytes) }),
                    Err(why) => {
                        let msg = format!("<lottie src=\"{src}\">: could not be fetched — {why}");
                        job.slot().loaded = Some(Err(("lottie-src", msg)));
                    }
                }
                PENDING.fetch_sub(1, Ordering::SeqCst);
            })
            .detach();
        return Ok(());
    }
    let from = match crate::img::readable("lottie", trimmed)? {
        crate::img::Readable::Bytes(bytes) => From::Bytes(bytes),
        crate::img::Readable::File(path) => From::File(path),
    };
    submit(Work::Load { job: job.clone(), src: src.to_string(), from });
    Ok(())
}
// ---------------------------------------------------------------------------
// props
// ---------------------------------------------------------------------------

/// `loop` — browser boolean-attribute grammar, plus lottie-web's numeric form.
///
/// Absent is **`LoopMode::None`**: the `<lottie-player>` default, not
/// lottie-web's (decision 1 in the module docs). `:loop="n"` gets the `+ 1`
/// that turns the crate's "play n times" into lottie-web's "repeat n times"
/// (decision 3).
fn loop_mode_from(raw: Option<&str>) -> LoopMode {
    match raw.map(str::trim) {
        None | Some("false") => LoopMode::None,
        Some("") | Some("true") => LoopMode::Loop,
        Some(v) => match v.parse::<f64>() {
            // `loop="Infinity"` parses, and so does a count past `u32`: both
            // loop forever, where `as u32 + 1` overflowed to a count of 0.
            Ok(n) if n >= u32::MAX as f64 => LoopMode::Loop,
            Ok(n) if n >= 1.0 => LoopMode::Count(n as u32 + 1),
            // `loop="0"` is falsy in lottie-web, like `loop: 0`.
            Ok(_) => LoopMode::None,
            // Any other non-empty attribute value is truthy in HTML.
            Err(_) => LoopMode::Loop,
        },
    }
}

/// The browser's boolean-attribute rule: present is true unless it spells
/// `"false"` (which is what `:attr="false"` serialises to through `patchProp`).
fn bool_attr(raw: Option<&str>) -> bool {
    raw.is_some_and(|v| v.trim() != "false")
}

/// `segment="in,out"`. An array binding (`:segment="[10, 40]"`) cannot reach
/// here — `patchProp` drops non-primitive props with its own advisory — so the
/// string spelling is the supported one, and it is what gets documented.
fn segment_from(raw: Option<&str>) -> Option<(f32, f32)> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let (a, b) = match raw.split_once(',') {
        Some(pair) => pair,
        None => raw.split_once(char::is_whitespace)?,
    };
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Props lottie-web / `<lottie-player>` have and this renderer does not.
const UNSUPPORTED_PROPS: &[(&str, &str)] = &[
    ("renderer", "rendering is always the software rasteriser; there is no svg/canvas/html renderer to choose"),
    ("background", "use the CSS `background-color` / `background-image` on the element — the raster is transparent, so it shows through"),
    ("controls", "no built-in transport UI; build one in Vue and drive `autoplay` / `speed` / `direction`"),
    ("hover", "no hover-to-play; bind `autoplay` to a ref and flip it from @mouseenter / @mouseleave"),
    ("intermission", "no pause between loops"),
    ("count", "use `loop=\"<n>\"`, which is lottie-web's spelling of the same thing"),
    ("preserveaspectratio", "use the CSS `object-fit` property (or the `objectfit` attribute)"),
    ("webworkers", "rasterisation always runs on a worker thread; there is nothing to switch"),
    ("subframe", "subframe rendering is always on — the playhead is continuous and is never snapped to source frames"),
];

fn warn_unsupported_props(t: &Tree, id: u32) {
    for (name, alternative) in UNSUPPORTED_PROPS {
        if t.prop(id, name).is_some() {
            crate::render::warn_compat(
                &format!("lottie-prop:{name}"),
                &format!("<lottie {name}=…> is not supported — {alternative}"),
            );
        }
    }
}


// ---------------------------------------------------------------------------
// lifecycle
// ---------------------------------------------------------------------------

/// Starts the load if `src` changed (or there is no entry yet), then pushes
/// every prop into the transport. Returns the animation's intrinsic size,
/// which is what the box falls back to when CSS declares none — `None` while
/// nothing is loaded, i.e. the element paints nothing at all.
///
/// Called from the render walk, once per node per frame. The load itself is
/// off the UI thread; the rest is a handful of setters the transport dedupes.
pub(crate) fn sync(t: &Tree, id: u32, win: WinKey, cx: &mut App) -> Option<(u32, u32)> {
    warn_unsupported_props(t, id);
    let src = t.prop(id, "src").unwrap_or("").to_string();
    let autoplay = bool_attr(t.prop(id, "autoplay"));

    LOTTIE_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let fresh = map.get(&id).is_none_or(|entry| entry.src != src);
        if fresh {
            // The old picture goes back to its atlas; the old load is dropped
            // wherever it is.
            if let Some(old) = map.remove(&id) {
                forget(old);
            }
            let entry = Entry::new(src.clone(), autoplay, win);
            // An absent `src` is not an error worth shouting about (a `v-if`'d
            // or not-yet-resolved binding hits this on its first frame).
            if !src.trim().is_empty() {
                if let Err(msg) = start_load(&entry.job, &src, cx) {
                    crate::render::warn_compat(&format!("lottie-src:{src}"), &msg);
                }
            }
            map.insert(id, entry);
        }

        let entry = map.get_mut(&id)?;
        // Re-stamped every frame: a node can move between windows over its life
        // (pop out, come back), and its tile belongs to the window that built it.
        entry.window = win;
        apply_props(t, id, entry, false, autoplay);
        let meta = entry.transport.as_ref()?.meta();
        Some((meta.width, meta.height))
    })
}

/// Pushes the props into a loaded node's transport. `loaded` is the frame its
/// load landed on: then `autoplay` decides whether it starts at all (the
/// transport starts playing); afterwards a change is a play/pause command,
/// which is what makes `:autoplay="isPlaying"` work as the transport control
/// markup needs it to be. `<lottie-player>` only reads `autoplay` at load and
/// exposes `.play()` / `.pause()` instead — we have no element methods, so
/// this is a deliberate, documented superset.
fn apply_props(t: &Tree, id: u32, entry: &mut Entry, loaded: bool, autoplay: bool) {
    let Some(state) = entry.transport.as_mut() else {
        // Still loading: the value seen on the frame it lands is the one that
        // decides.
        entry.autoplay = autoplay;
        return;
    };
    state.set_speed(
        t.prop(id, "speed")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(1.0),
    );
    state.set_direction(match t.prop(id, "direction").map(str::trim) {
        Some("-1") | Some("reverse") => Direction::Reverse,
        _ => Direction::Forward,
    });
    state.set_play_mode(match t.prop(id, "mode").map(str::trim) {
        Some("bounce") => PlayMode::Bounce,
        _ => PlayMode::Normal,
    });
    state.set_segment(segment_from(t.prop(id, "segment")));

    // `set_loop_mode` clears `finished` for any looping mode, so flipping
    // `:loop` on a completed animation makes it run again — lottie-web's
    // `setLoop` does the same.
    state.set_loop_mode(loop_mode_from(t.prop(id, "loop")));

    if loaded || autoplay != entry.autoplay {
        if autoplay {
            state.play();
        } else {
            state.pause();
        }
    }
    entry.autoplay = autoplay;
}

/// What the build paints for a node laid into a box of `size` logical px on
/// a `scale` display: its tile, once it has a frame in this window. Records
/// the box, which is what the next raster is drawn for.
pub(crate) fn paint(id: u32, win: WinKey, size: (f32, f32), scale: f32) -> Option<Arc<RenderImage>> {
    LOTTIE_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let entry = map.get_mut(&id)?;
        entry.boxed = Some((size, scale));
        entry.tile.as_ref().filter(|tile| tile.live.window() == win).map(|tile| tile.live.image())
    })
}

/// Lets go of a node's state: its tile goes back to its atlas, and the worker
/// drops whatever it still holds for it.
fn forget(entry: Entry) {
    let mut slot = entry.job.slot();
    slot.gone = true;
    slot.animation = None;
    slot.drawn = None;
    drop(slot);
    if let Some(tile) = entry.tile {
        tile.live.retire();
    }
}

/// Drops the entries of nodes that no longer exist. Their tiles go back to
/// their windows' atlases on those windows' next frames (CONTRACT.md §8.9).
pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    let gone: Vec<Entry> = LOTTIE_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let ids: Vec<u32> = map.keys().copied().filter(|&id| !live(id)).collect();
        ids.into_iter().filter_map(|id| map.remove(&id)).collect()
    });
    for entry in gone {
        forget(entry);
    }
}

/// A window closed: its atlas went with it, so its tiles are simply dropped.
pub(crate) fn forget_window(key: WinKey) {
    LOTTIE_STATES.with(|m| {
        for entry in m.borrow_mut().values_mut() {
            if entry.tile.as_ref().is_some_and(|tile| tile.live.window() == key) {
                entry.tile = None;
                entry.asked = None;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// the frame pump
// ---------------------------------------------------------------------------

/// True while any `<lottie>` still needs frames — folded into the host's
/// `animating()` OR-chain (CONTRACT.md §4) — or a load or a raster is out on
/// the worker, whose result the next tick takes.
pub fn any_playing() -> bool {
    PENDING.load(Ordering::SeqCst) > 0
        || LOTTIE_STATES.with(|m| {
            m.borrow().values().any(|e| {
                e.transport.as_ref().is_some_and(Transport::needs_frame)
                    || e.job.slot().loaded.is_some()
            })
        })
}

/// One tick's events for one node, with what their payloads read.
#[derive(Clone, Copy)]
struct Ticked {
    id: u32,
    advance: Advance,
    total_frames: f32,
    frame: f32,
    progress: f32,
}

/// Takes the loads that landed, advances every `<lottie>` by the wall-clock
/// delta since the previous frame, asks the worker for the frames the nodes
/// that can be seen now show, writes the frames it drew into this window's
/// tiles, and dispatches the events. Returns `true` when the window must draw:
/// a box that takes its animation's size, a tile's first frame, a new tile.
/// A frame written in place asks only for a present (`crate::live`).
///
/// Runs from the host's frame pump, **before** the draw.
pub fn tick(window: &mut Window, cx: &mut App) -> bool {
    drain_pending_drops(window, cx);

    let now = Instant::now();
    let dt = LAST_TICK
        .with(|t| t.replace(Some(now)))
        .map(|prev| (now - prev).min(MAX_DT))
        .unwrap_or_default();
    let here = windows::key(window);
    let page_hidden = live::page_hidden();
    let here_shown = window.is_visible();
    let mut draw = false;
    let mut touch: Vec<u32> = Vec::new();

    // Loads that landed. Their transports start now, with the props the node
    // has now, and move from the next tick on.
    let mut landed: Vec<(u32, String, Result<LottieMeta, (&'static str, String)>, Vec<String>)> = Vec::new();
    LOTTIE_STATES.with(|m| {
        for (&id, entry) in m.borrow_mut().iter_mut() {
            let mut slot = entry.job.slot();
            if let Some(loaded) = slot.loaded.take() {
                let advisories = std::mem::take(&mut slot.advisories);
                drop(slot);
                if let Ok(meta) = &loaded {
                    entry.transport = Some(Transport::new(meta.clone()));
                    // The box takes the animation's size: build it again.
                    touch.push(id);
                    draw = true;
                }
                landed.push((id, entry.src.clone(), loaded, advisories));
            }
        }
    });
    if !landed.is_empty() {
        tree::read_tree(|t| {
            LOTTIE_STATES.with(|m| {
                let mut map = m.borrow_mut();
                for (id, ..) in landed.iter().filter(|(.., loaded, _)| loaded.is_ok()) {
                    if let Some(entry) = map.get_mut(id) {
                        let autoplay = bool_attr(t.prop(*id, "autoplay"));
                        apply_props(t, *id, entry, true, autoplay);
                    }
                }
            })
        });
        for (_, src, loaded, advisories) in &landed {
            report_load(src, loaded, advisories);
        }
    }
    let fresh: Vec<u32> = landed.iter().map(|(id, ..)| *id).collect();

    // The clocks. Collect first, dispatch after: a JS handler can mutate the
    // tree (and `flush_changes` re-enters `gc`), so the map must not be
    // borrowed while any of this reaches JavaScript.
    let ticks: Vec<Ticked> = if dt.is_zero() {
        Vec::new()
    } else {
        LOTTIE_STATES.with(|m| {
            m.borrow_mut()
                .iter_mut()
                .filter(|(id, _)| !fresh.contains(id))
                .filter_map(|(&id, entry)| {
                    let state = entry.transport.as_mut()?;
                    let advance = state.advance(dt);
                    let interesting = advance.needs_repaint
                        || advance.completed
                        || advance.loops_completed > 0
                        || advance.entered_frame.is_some();
                    interesting.then(|| Ticked {
                        id,
                        advance,
                        total_frames: state.meta().total_frames,
                        frame: state.frame(),
                        progress: state.progress(),
                    })
                })
                .collect()
        })
    };

    // The rasters: written when drawn, asked for when a node that can be seen
    // shows another frame or size than its tile holds.
    let shown = write(window, here, &mut touch, &mut draw);
    live::note(shown);
    LOTTIE_STATES.with(|m| {
        for (&id, entry) in m.borrow_mut().iter_mut() {
            let Some((key, frame)) = entry.want() else { continue };
            let visible = !page_hidden && (entry.window != here || here_shown) && live::visible(id, entry.window);
            let holds = entry.tile.as_ref().is_some_and(|tile| tile.shows == key && tile.live.window() == entry.window);
            if !visible || holds || entry.asked == Some(key) {
                continue;
            }
            entry.asked = Some(key);
            ask(&entry.job, frame, key.1, key.2);
        }
    });

    if !touch.is_empty() {
        tree::with_tree(|t| {
            for id in touch {
                t.touch(id);
            }
        });
    }
    for ticked in ticks {
        dispatch_events(&ticked, window, cx);
    }
    draw
}

/// Writes the frames the worker drew for nodes built in window `here` into
/// their tiles. Another window's frames wait for its own draw
/// ([`window_tick`]), which `draw` asks for.
fn write(window: &mut Window, here: WinKey, touch: &mut Vec<u32>, draw: &mut bool) -> Shown {
    let mut shown = Shown::Nowhere;
    LOTTIE_STATES.with(|m| {
        for (&id, entry) in m.borrow_mut().iter_mut() {
            if entry.window != here {
                *draw |= entry.job.slot().drawn.is_some();
                continue;
            }
            let Some((key, bytes)) = entry.job.slot().drawn.take() else { continue };
            let (_, w, h) = key;
            let fits = entry.tile.as_ref().is_some_and(|tile| tile.live.window() == here && tile.live.size() == (w, h));
            if !fits {
                // Another size or window: a new tile, which the node paints
                // from its next build on; the old one goes back then.
                if let Some(old) = entry.tile.take() {
                    old.live.retire();
                }
                entry.tile = Some(Tile { live: LiveTile::new(here, w, h), shows: key });
            }
            let tile = entry.tile.as_mut().expect("just made");
            tile.shows = key;
            // Asked as if visible, to tell a write in place from one that
            // moved the tile.
            let wrote = tile.live.write_bgra(window, bytes, true);
            if !fits || wrote == Shown::Draw {
                // A cache boundary over the node would replay a sprite that
                // points at the old place.
                touch.push(id);
                shown = Shown::Draw;
            } else {
                let visible = live::visible(id, here);
                shown = shown.max(if visible { wrote } else { Shown::Nowhere });
            }
        }
    });
    shown
}

/// A pop-out's half of [`tick`], from its own draw: its atlas's dropped
/// tiles handed back, and the frames drawn for the nodes it built written
/// into its tiles. The clocks and the events are the pump's, once per frame
/// group (lib.rs `window_frame_tick`).
pub(crate) fn window_tick(window: &mut Window, cx: &mut App) {
    drain_pending_drops(window, cx);
    let here = windows::key(window);
    let mut touch = Vec::new();
    let mut draw = false;
    write(window, here, &mut touch, &mut draw);
    if !touch.is_empty() {
        tree::with_tree(|t| {
            for id in touch {
                t.touch(id);
            }
        });
    }
}

fn drain_pending_drops(window: &mut Window, cx: &mut App) {
    let images = crate::windows::here(window, |w| std::mem::take(&mut w.pending_drops));
    for image in images {
        cx.drop_image(image, Some(window));
    }
}

/// A load's outcome on the compat channel: what the compatibility pass
/// rewrote, or why it failed and which features it lacks.
fn report_load(src: &str, loaded: &Result<LottieMeta, (&'static str, String)>, advisories: &[String]) {
    match loaded {
        Ok(_) => {
            // A *successful* load can still have had to rewrite the document.
            for advisory in advisories {
                crate::render::warn_compat(
                    &format!("lottie-load:{src}:{advisory}"),
                    &format!("<lottie src=\"{src}\">: {advisory}"),
                );
            }
        }
        Err((key, msg)) => {
            crate::render::warn_compat(&format!("{key}:{src}"), msg);
            for advisory in advisories {
                crate::render::warn_compat(
                    &format!("lottie-unsupported:{src}:{advisory}"),
                    &format!("<lottie src=\"{src}\">: unsupported feature — {advisory}"),
                );
            }
        }
    }
}

/// One tick's events, in lottie-web's firing order, gated on the listener bits.
///
/// **The `loopComplete` that coincides with `complete` is dropped** — decision 2
/// in the module docs. `Advance::events()` reports the boundary that ended
/// playback as both, because at the crate's level it *is* both; lottie-web
/// fires only `complete` there, for `loop: false` and for a numeric `loop`
/// alike.
fn dispatch_events(ticked: &Ticked, window: &mut Window, cx: &mut App) {
    let Ticked { id, advance, total_frames, frame, progress } = *ticked;
    let listeners = tree::read_tree(|t| t.node(id).map(|n| n.listeners)).unwrap_or(0);
    let listens = |ev: u32| listeners & (1u64 << ev) != 0;
    if listeners == 0 {
        return;
    }

    // `loops_completed` counts boundaries crossed this tick; the last of them
    // is the completing one when `completed` is set.
    let suppressed_loop = advance.completed.then_some(advance.loop_count);
    // lottie-web's `playCount` at `complete` does not count the completing
    // pass, so neither does the payload.
    let loops_done = if advance.completed {
        advance.loop_count.saturating_sub(1)
    } else {
        advance.loop_count
    };
    let total_frames = total_frames as f64;

    for event in advance.events() {
        match event {
            LottieEvent::EnterFrame { frame, elapsed } => {
                // High frequency: do not even build the payload unless someone
                // is listening (CONTRACT.md §2).
                if !listens(tree::EV_ENTERFRAME) {
                    continue;
                }
                let payload = EventPayload {
                    a: frame as f64,
                    b: total_frames,
                    c: elapsed as f64,
                    d: progress as f64,
                    key: None,
                };
                crate::render::js_dispatch_app(id, tree::EV_ENTERFRAME, &payload, window, cx);
            }
            LottieEvent::LoopComplete { count } => {
                if suppressed_loop == Some(count) || !listens(tree::EV_LOOPCOMPLETE) {
                    continue;
                }
                let payload = EventPayload {
                    a: frame as f64,
                    b: total_frames,
                    c: count as f64,
                    ..Default::default()
                };
                crate::render::js_dispatch_app(id, tree::EV_LOOPCOMPLETE, &payload, window, cx);
            }
            LottieEvent::Complete => {
                if !listens(tree::EV_COMPLETE) {
                    continue;
                }
                let payload = EventPayload {
                    a: frame as f64,
                    b: total_frames,
                    c: loops_done as f64,
                    ..Default::default()
                };
                crate::render::js_dispatch_app(id, tree::EV_COMPLETE, &payload, window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_follows_the_lottie_player_defaults_and_lottie_webs_counting() {
        // OWNER DECISION 1: absent means NOT looping (`<lottie-player>`), not
        // lottie-web's `loadAnimation` default.
        assert_eq!(loop_mode_from(None), LoopMode::None);
        assert_eq!(loop_mode_from(Some("false")), LoopMode::None);
        // Browser boolean-attribute spellings.
        assert_eq!(loop_mode_from(Some("")), LoopMode::Loop);
        assert_eq!(loop_mode_from(Some("true")), LoopMode::Loop);
        // OWNER DECISION 3: lottie-web's numeric `loop` counts repeats, so
        // `loop="2"` plays three times — Count(3) in the crate's counting.
        assert_eq!(loop_mode_from(Some("2")), LoopMode::Count(3));
        assert_eq!(loop_mode_from(Some("0")), LoopMode::None);
        assert_eq!(loop_mode_from(Some("Infinity")), LoopMode::Loop);
        assert_eq!(loop_mode_from(Some("1e10")), LoopMode::Loop);
    }

    #[test]
    fn boolean_attributes_follow_the_html_rule() {
        assert!(!bool_attr(None));
        assert!(bool_attr(Some("")));
        assert!(bool_attr(Some("true")));
        assert!(!bool_attr(Some("false")));
    }

    #[test]
    fn segments_parse_from_the_string_spelling() {
        assert_eq!(segment_from(Some("10,40")), Some((10.0, 40.0)));
        assert_eq!(segment_from(Some(" 10 , 40 ")), Some((10.0, 40.0)));
        assert_eq!(segment_from(None), None);
        assert_eq!(segment_from(Some("")), None);
        assert_eq!(segment_from(Some("nonsense")), None);
    }
}
