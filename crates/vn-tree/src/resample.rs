//! Smaller copies of images drawn well below their own size (m26 client sweep
//! #22: 1080 px avatars at 40 px).
//!
//! gpui's sprite atlas keeps ONE mip level and samples it linearly, so a
//! picture drawn at a small fraction of its size reads a 2×2 neighbourhood out
//! of every N×N block and aliases: fine detail turns to noise, hair to
//! sparkle. A browser draws such an image from a mipmap or a filtered
//! downscale. Here the fork asks this module first (`gpui::ImageResampler`,
//! called from `Window::paint_image` with the size in device pixels the frame
//! is about to be drawn at), and this module answers with a copy filtered
//! down to that size: 2×2 box steps while the image is more than twice the
//! target, then Lanczos3 (the `image` crate's), in premultiplied floating
//! point so a transparent pixel adds no colour.
//!
//! The hook point is paint, not decode, because only paint knows the size an
//! image is drawn at — after `object-fit`, `background-size` and the window's
//! scale factor — and because a smaller decode would also shrink the image's
//! intrinsic size, which `<img>` layout reads. `<img>`, CSS `background-image`,
//! inline `<svg>` rasters and a still `<canvas>` all paint through it.
//!
//! # Which copy
//!
//! * Drawn at 0.8 of its size or more along an axis: that axis is not
//!   resampled; the atlas's bilinear sampling is fine there. Drawn larger: the
//!   image itself, always.
//! * Otherwise the size is rounded UP to a step of a quarter octave
//!   (2^(-k/4) of the natural size), so the sampler never shrinks a copy by
//!   more than about 16 %, and a size that wobbles by a pixel reuses its copy.
//! * The FIRST copy of an image is made at once: on the painting thread when
//!   the image is small enough to take a couple of milliseconds
//!   ([`SYNC_PIXELS`]: the avatar case, which therefore never shows aliased)
//!   and the frame has not spent [`SYNC_BUDGET`] on copies yet; on the
//!   resampling thread otherwise (the image is drawn as it is until the copy
//!   lands, and the frame tick repaints then).
//! * A copy for ANOTHER size of an image that already has one is made on the
//!   resampling thread once that size has been asked for [`SETTLE`]; until
//!   then the nearest existing copy is drawn. So an animation that resizes
//!   images never pays for copies in its frames, and the size steps bound how
//!   many it can cause.
//! * Copies are kept per (image, frame, size step), least recently drawn
//!   evicted past [`BUDGET_BYTES`], and a copy whose image is gone (gpui's
//!   cache let go of it, a canvas drew again) goes too. An evicted copy's atlas
//!   tile goes back to gpui from the frame tick.
//! * A frame on which a copy landed or was evicted replays no retained-render
//!   boundary ([`holds_replays`], `crate::cache`): a replayed subtree's scene
//!   still names the tile it recorded, which may be the one just freed, or the
//!   full-size image a new copy replaces. Both are rare events.
//!
//! Frames of a moving picture (`<video>`, `<lottie>`, a canvas that is being
//! redrawn) are `RenderImage::dynamic` and never reach this module.
//!
//! # The copy instead of the decode
//!
//! A copy that covers every size an image is drawn at makes the full decode
//! dead weight: a 1080 px avatar drawn at 36 px keeps 4.4 MB of pixels to
//! draw from a 23 KB copy (research/reports/m26/perf-mem-2.md §3). So this
//! module also records how each image is drawn ([`note_use`]), and
//! `crate::img`'s residency sweep asks [`reduced_copy`] for a copy to put in
//! the decode's place in gpui's asset cache. A copy carries the picture's
//! natural size (`RenderImage::with_natural_size`), so layout, `object-fit`
//! and `naturalWidth` cannot tell. A reduced image drawn larger than its
//! pixels asks for the full decode back ([`take_wanted_full`]); it is drawn
//! from the copy, softer, until the decode lands.
//!
//! **Divergence (IMG-1, docs/RUNTIME.md §5.16):** the size is the box's, not
//! the one a CSS `transform: scale()` gives it on screen.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use gpui::{DevicePixels, ImageId, RenderImage, Size};
// The filter itself is its own crate, so debug builds optimise it.
use vn_resample::downscale;

/// Below this fraction of its size along an axis, an image is drawn from a copy.
const RESAMPLE_BELOW: f32 = 0.8;
/// Size steps per octave.
const STEPS: f32 = 4.0;
/// The largest image (in source pixels) whose first copy is made on the
/// painting thread: a 1080 × 1080 avatar is 1.17 M, and its copy takes about
/// 1.5 ms (`vn_resample`'s `an_avatar_and_a_banner_downscale_quickly`; the
/// crate is optimised in debug builds too).
const SYNC_PIXELS: u64 = 1_300_000;
/// What one frame may spend making first copies on the painting thread. A
/// list of new avatars past it gets the rest of its copies from the
/// resampling thread a frame or two later, instead of one long frame.
const SYNC_BUDGET: Duration = Duration::from_millis(4);
/// How long another size of an image must be asked for before it gets a copy.
const SETTLE: Duration = Duration::from_millis(100);
/// The bytes of copies kept.
const BUDGET_BYTES: usize = 64 << 20;
/// How long an image must have been painted before a copy may stand in for
/// its decode: long enough for the sizes it is drawn at to settle and their
/// copies to exist.
const REDUCE_AFTER: Duration = Duration::from_secs(2);
/// The span of each of the two windows the largest drawn size is kept over: a
/// large draw (a profile card, a lightbox) holds the full decode for 15–30 s
/// after it ends, so opening it again soon does not load the image again.
const USE_WINDOW: Duration = Duration::from_secs(15);
/// A copy stands in for a decode only when that frees at least this much and
/// at least three quarters of the decode.
const MIN_SAVING: usize = 256 << 10;
/// How much larger than its pixels a reduced image may be drawn before the
/// full decode is wanted back (the quarter-octave step a copy is rounded up to).
const UPSCALE_TOLERANCE: f32 = 1.2;

/// A copy's key: the image, its frame, and the size step on each axis (0 = the
/// natural size on that axis, -1 = 2^(-1/4) of it, …).
type Key = (ImageId, usize, i32, i32);

struct Copy {
    image: Arc<RenderImage>,
    source: Weak<RenderImage>,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Cache {
    copies: HashMap<Key, Copy>,
    in_flight: HashSet<Key>,
    /// Other sizes of images that have a copy, and since when they are asked for.
    waiting: HashMap<Key, (Weak<RenderImage>, Instant)>,
    clock: u64,
    bytes: usize,
    /// Time spent making copies on the painting thread since the last tick.
    sync_spent: Duration,
    /// Evicted copies, for the frame tick to hand back to gpui's atlases.
    retired: Vec<Arc<RenderImage>>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    static HOLD_REPLAYS: Cell<bool> = const { Cell::new(false) };
    /// How each painted image has been drawn lately ([`note_use`]).
    static USES: RefCell<HashMap<ImageId, Use>> = RefCell::new(HashMap::new());
    /// Reduced images drawn larger than their pixels since the last sweep.
    static WANT_FULL: RefCell<HashSet<ImageId>> = RefCell::new(HashSet::new());
}

/// The sizes one image was drawn at, in device pixels.
struct Use {
    image: Weak<RenderImage>,
    /// When it was first drawn.
    first: Instant,
    /// The largest size drawn in the current window and in the one before it.
    cur: (i32, i32),
    prev: (i32, i32),
    rotated: Instant,
    /// The size of the latest draw: what an image stands for once a retained
    /// subtree replays it (a replay draws without asking this module).
    last: (i32, i32),
}

/// Records a draw of `image` at `device` pixels, and a reduced image drawn
/// well above its pixels as wanting its full decode back.
fn note_use(image: &Arc<RenderImage>, device: Size<DevicePixels>, now: Instant) {
    let d = (device.width.0, device.height.0);
    if image.is_reduced() {
        let px = image.size(0);
        let over = |d: i32, p: i32| d as f32 > p as f32 * UPSCALE_TOLERANCE;
        if over(d.0, px.width.0) || over(d.1, px.height.0) {
            WANT_FULL.with(|w| w.borrow_mut().insert(image.id));
        }
    }
    USES.with(|u| {
        let mut u = u.borrow_mut();
        let e = u.entry(image.id).or_insert_with(|| Use {
            image: Arc::downgrade(image),
            first: now,
            cur: (0, 0),
            prev: (0, 0),
            rotated: now,
            last: d,
        });
        let since = now.duration_since(e.rotated);
        if since >= USE_WINDOW {
            // A window that ended more than a window ago holds nothing recent.
            e.prev = if since >= USE_WINDOW * 2 { (0, 0) } else { e.cur };
            e.cur = (0, 0);
            e.rotated = now;
        }
        e.cur = (e.cur.0.max(d.0), e.cur.1.max(d.1));
        e.last = d;
    });
}

/// The largest size `id` was drawn at lately, and how long it has been drawn.
fn recent_use(id: ImageId, now: Instant) -> Option<((i32, i32), Duration)> {
    USES.with(|u| {
        let u = u.borrow();
        let e = u.get(&id)?;
        // Windows that ended with no draw since count as empty.
        let since = now.duration_since(e.rotated);
        let (cur, prev) = if since >= USE_WINDOW * 2 {
            ((0, 0), (0, 0))
        } else if since >= USE_WINDOW {
            ((0, 0), e.cur)
        } else {
            (e.cur, e.prev)
        };
        let w = cur.0.max(prev.0).max(e.last.0);
        let h = cur.1.max(prev.1).max(e.last.1);
        Some(((w, h), now.duration_since(e.first)))
    })
}

/// Why [`reduced_copy`] left a decode full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeepFull {
    /// Already a copy, or not a still picture (an animation, a moving frame).
    NotStill,
    /// Not drawn since it was decoded (a preload, a hidden element).
    Undrawn,
    /// Drawn for less than [`REDUCE_AFTER`].
    Young,
    /// Drawn at 0.8 of its size or more along an axis lately.
    DrawnLarge,
    /// No copy covers the largest size it was drawn at (yet).
    NoCopy,
    /// The copy would free less than [`MIN_SAVING`] or a quarter of it.
    SmallSaving,
}

/// A copy of `full` that covers every size it was drawn at lately, taken out
/// of the copy cache (its atlas tile stays: it is drawn from next) for the
/// caller to put in the decode's place — or why the decode has to stay.
pub(crate) fn reduced_copy(full: &Arc<RenderImage>, now: Instant) -> Result<Arc<RenderImage>, KeepFull> {
    if full.is_reduced() || full.is_dynamic() || full.frame_count() != 1 {
        return Err(KeepFull::NotStill);
    }
    let (drawn, age) = recent_use(full.id, now).ok_or(KeepFull::Undrawn)?;
    if drawn.0 <= 0 || drawn.1 <= 0 {
        return Err(KeepFull::Undrawn);
    }
    if age < REDUCE_AFTER {
        return Err(KeepFull::Young);
    }
    let natural = full.size(0);
    let (nw, nh) = (natural.width.0, natural.height.0);
    if nw <= 0 || nh <= 0 {
        return Err(KeepFull::NotStill);
    }
    let need = (step(drawn.0 as f32 / nw as f32), step(drawn.1 as f32 / nh as f32));
    if need.0 == 0 || need.1 == 0 {
        return Err(KeepFull::DrawnLarge);
    }
    let full_bytes = nw as usize * nh as usize * 4;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        // The smallest copy at least as large as the largest draw: steps at or
        // above the needed ones, the lowest of those.
        let key = c
            .copies
            .keys()
            .filter(|k| k.0 == full.id && k.1 == 0 && k.2 >= need.0 && k.3 >= need.1)
            .min_by_key(|k| (k.2 + k.3, k.2))
            .copied()
            .ok_or(KeepFull::NoCopy)?;
        let bytes = c.copies.get(&key).ok_or(KeepFull::NoCopy)?.bytes;
        if bytes.saturating_mul(4) > full_bytes || full_bytes - bytes < MIN_SAVING {
            return Err(KeepFull::SmallSaving);
        }
        let copy = c.copies.remove(&key).ok_or(KeepFull::NoCopy)?;
        c.bytes -= copy.bytes;
        Ok(copy.image)
    })
}

/// The ids of reduced images drawn larger than their pixels since the last call.
pub(crate) fn take_wanted_full() -> HashSet<ImageId> {
    WANT_FULL.with(|w| std::mem::take(&mut *w.borrow_mut()))
}

/// Marks a reduced image as wanting its full decode back (an `<img>` whose
/// pixels a canvas reads).
pub(crate) fn want_full(id: ImageId) {
    WANT_FULL.with(|w| w.borrow_mut().insert(id));
}

/// Forgets the draws of images that are gone.
pub(crate) fn prune_uses() {
    USES.with(|u| u.borrow_mut().retain(|_, e| e.image.strong_count() > 0));
}

/// Something other than this module swapped or freed an atlas tile this
/// frame: no retained-render boundary may replay (see [`holds_replays`]).
pub(crate) fn hold_replays_too() {
    HOLD_REPLAYS.with(|h| h.set(true));
}

/// One copy to make on the resampling thread.
struct Job {
    key: Key,
    source: Arc<RenderImage>,
    target: (u32, u32),
}

type Landed = (Key, Weak<RenderImage>, Option<RenderImage>);

static DONE: Mutex<Vec<Landed>> = Mutex::new(Vec::new());
static JOBS: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();

/// `VN_NO_RESAMPLE=1` leaves every image to the atlas's own sampling: the A/B
/// for `specs/resample.spec.ts`, and a switch for a field diagnosis.
fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let off = std::env::var("VN_NO_RESAMPLE").map(|v| {
            let v = v.trim().to_ascii_lowercase();
            !(v.is_empty() || v == "0" || v == "false" || v == "off")
        });
        !off.unwrap_or(false)
    })
}

/// An image its owner is done with: its copies go back to gpui at the next tick.
pub(crate) fn forget(image: &RenderImage) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let keys: Vec<Key> = c.copies.keys().filter(|k| k.0 == image.id).copied().collect();
        for k in keys {
            evict(&mut c, k);
        }
        c.waiting.retain(|k, _| k.0 != image.id);
    });
}

/// Did a copy land or go this frame? Then no retained-render boundary may
/// replay (see the module docs).
pub(crate) fn holds_replays() -> bool {
    HOLD_REPLAYS.with(Cell::get)
}

/// The size step for a scale along one axis: 0 when the axis is drawn at
/// [`RESAMPLE_BELOW`] of its size or more, else the step whose size is the
/// smallest at or above the drawn one.
pub(crate) fn step(scale: f32) -> i32 {
    if !(scale > 0.0) || scale >= RESAMPLE_BELOW {
        return 0;
    }
    ((scale.log2() * STEPS).ceil() as i32).min(-1)
}

/// The pixel size a step gives a natural size.
pub(crate) fn stepped(natural: u32, step: i32) -> u32 {
    if step == 0 {
        return natural;
    }
    ((natural as f32 * (step as f32 / STEPS).exp2()).ceil() as u32).clamp(1, natural)
}

/// `gpui::ImageResampler`: the copy to draw `image`'s `frame` from at
/// `device` pixels, or `None` to draw it as it is.
fn resampler(image: &Arc<RenderImage>, frame: usize, device: Size<DevicePixels>) -> Option<Arc<RenderImage>> {
    note_use(image, device, Instant::now());
    let out = choose(image, frame, device);
    if TRACK.load(std::sync::atomic::Ordering::Relaxed) {
        note_seen(image, device, out.is_none());
    }
    out
}

// ---------------------------------------------------------------------------
// the painted-image registry (devtools `mem`, VN_MEMSTATS)
// ---------------------------------------------------------------------------
//
// gpui keeps every decoded image in its asset cache and gives nobody outside
// it a way to list them. Every one that is PAINTED passes through the hook
// above, though, with the size it is drawn at — which is the number the
// "decoded at full size, drawn at 40 px" question needs. So, while armed, the
// hook records each image it sees: a `Weak` (which keeps no pixels alive), its
// natural size, the largest size it was drawn at, and whether it was ever
// drawn from the full-size image itself (which puts the full image into the
// polychrome atlas, where it stays until gpui is told to drop it).

static TRACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Seen {
    image: Weak<RenderImage>,
    /// Decoded bytes over all frames (BGRA, 4 B/px).
    bytes: usize,
    natural: (i32, i32),
    /// The largest device-pixel size any frame was drawn at.
    drawn: (i32, i32),
    /// Drawn at least once from the full-size image (no copy).
    full_drawn: bool,
}

thread_local! {
    static SEEN: RefCell<HashMap<ImageId, Seen>> = RefCell::new(HashMap::new());
}

/// Arms the registry (`VN_MEMSTATS=1`, read by the host at boot).
pub fn set_tracking(on: bool) {
    TRACK.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn note_seen(image: &Arc<RenderImage>, device: Size<DevicePixels>, full: bool) {
    SEEN.with(|s| {
        let mut s = s.borrow_mut();
        let e = s.entry(image.id).or_insert_with(|| {
            let natural = image.size(0);
            let bytes = (0..image.frame_count())
                .map(|i| {
                    let z = image.size(i);
                    (z.width.0.max(0) as usize) * (z.height.0.max(0) as usize) * 4
                })
                .sum();
            Seen {
                image: Arc::downgrade(image),
                bytes,
                natural: (natural.width.0, natural.height.0),
                drawn: (0, 0),
                full_drawn: false,
            }
        });
        e.drawn = (e.drawn.0.max(device.width.0), e.drawn.1.max(device.height.0));
        e.full_drawn |= full;
    });
}

/// What the painted-image registry and the copy cache hold right now.
#[derive(Default, Debug, Clone)]
pub struct ImageStats {
    /// Is the registry armed at all? (Everything below except `copies*` is 0 when not.)
    pub tracking: bool,
    /// Painted images whose decode is still alive (something — gpui's asset
    /// cache, a node's memo — still holds the `Arc`).
    pub live: usize,
    pub live_bytes: usize,
    /// The same images, if each were decoded at the largest size it was drawn
    /// at (no smaller than 1×1, never larger than its natural size).
    pub live_bytes_at_drawn: usize,
    /// Live images at least 4× the pixels they are drawn at.
    pub oversized: usize,
    pub oversized_bytes: usize,
    /// Images drawn at least once from the full-size decode — each of those put
    /// the full image into the polychrome atlas (estimate: a later
    /// `drop_image` is not seen here).
    pub full_in_atlas: usize,
    pub full_in_atlas_bytes: usize,
    /// Painted images whose decode has gone since.
    pub gone: usize,
    /// The downscaled copies this module keeps, and their bytes (budget
    /// [`BUDGET_BYTES`]).
    pub copies: usize,
    pub copies_bytes: usize,
    pub copies_in_flight: usize,
    /// The five largest live decodes: (natural w, h, drawn w, h, bytes).
    pub largest: Vec<(i32, i32, i32, i32, usize)>,
}

/// The registry and copy-cache totals (devtools `mem`). Prunes nothing: a
/// gone image stays counted under `gone`.
pub fn image_stats() -> ImageStats {
    let mut out = ImageStats { tracking: TRACK.load(std::sync::atomic::Ordering::Relaxed), ..Default::default() };
    CACHE.with(|c| {
        let c = c.borrow();
        out.copies = c.copies.len();
        out.copies_bytes = c.bytes;
        out.copies_in_flight = c.in_flight.len();
    });
    SEEN.with(|s| {
        let s = s.borrow();
        let mut largest: Vec<(i32, i32, i32, i32, usize)> = Vec::new();
        for e in s.values() {
            if e.full_drawn {
                out.full_in_atlas += 1;
                out.full_in_atlas_bytes += e.bytes;
            }
            if e.image.strong_count() == 0 {
                out.gone += 1;
                continue;
            }
            out.live += 1;
            out.live_bytes += e.bytes;
            let (nw, nh) = (e.natural.0.max(1) as usize, e.natural.1.max(1) as usize);
            let (dw, dh) = (e.drawn.0.clamp(1, nw as i32) as usize, e.drawn.1.clamp(1, nh as i32) as usize);
            // Scaled per frame by the same ratio as frame 0.
            let ratio = (dw * dh) as f64 / (nw * nh) as f64;
            out.live_bytes_at_drawn += (e.bytes as f64 * ratio) as usize;
            if nw * nh >= 4 * dw * dh {
                out.oversized += 1;
                out.oversized_bytes += e.bytes;
            }
            largest.push((e.natural.0, e.natural.1, e.drawn.0, e.drawn.1, e.bytes));
        }
        largest.sort_by(|a, b| b.4.cmp(&a.4));
        largest.truncate(5);
        out.largest = largest;
    });
    out
}

fn choose(image: &Arc<RenderImage>, frame: usize, device: Size<DevicePixels>) -> Option<Arc<RenderImage>> {
    let natural = image.size(frame);
    let (nw, nh) = (natural.width.0, natural.height.0);
    if nw <= 0 || nh <= 0 || device.width.0 <= 0 || device.height.0 <= 0 {
        return None;
    }
    let steps = (
        step(device.width.0 as f32 / nw as f32),
        step(device.height.0 as f32 / nh as f32),
    );
    if steps == (0, 0) {
        return None;
    }
    let key = (image.id, frame, steps.0, steps.1);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.clock += 1;
        let clock = c.clock;
        if let Some(copy) = c.copies.get_mut(&key) {
            copy.used = clock;
            return Some(copy.image.clone());
        }
        match nearest(&c, image.id, frame, steps) {
            None => {
                // The first copy: now, unless one is already on its way.
                if c.in_flight.iter().any(|k| k.0 == image.id && k.1 == frame) {
                    return None;
                }
                let target = (stepped(nw as u32, steps.0), stepped(nh as u32, steps.1));
                if (nw as u64) * (nh as u64) <= SYNC_PIXELS && c.sync_spent < SYNC_BUDGET {
                    let t = Instant::now();
                    let made = Arc::new(make(image, frame, target)?);
                    c.sync_spent += t.elapsed();
                    crate::perf::note_resample(Some(t.elapsed().as_secs_f32() * 1000.0));
                    insert(&mut c, key, made.clone(), Arc::downgrade(image), clock);
                    Some(made)
                } else {
                    c.in_flight.insert(key);
                    crate::perf::note_resample(None);
                    submit(Job { key, source: image.clone(), target });
                    None
                }
            }
            Some(near) => {
                // Another size: once it has been asked for a while.
                if !c.in_flight.contains(&key) {
                    c.waiting.entry(key).or_insert_with(|| (Arc::downgrade(image), Instant::now()));
                }
                let copy = c.copies.get_mut(&near)?;
                copy.used = clock;
                Some(copy.image.clone())
            }
        }
    })
}

/// The existing copy of `id`'s `frame` whose steps are closest to `steps`,
/// a larger one before a smaller one.
fn nearest(c: &Cache, id: ImageId, frame: usize, steps: (i32, i32)) -> Option<Key> {
    c.copies
        .keys()
        .filter(|k| k.0 == id && k.1 == frame)
        .min_by_key(|k| {
            let distance = (k.2 - steps.0).abs() + (k.3 - steps.1).abs();
            let smaller = k.2 < steps.0 || k.3 < steps.1;
            (distance, smaller)
        })
        .copied()
}

fn insert(c: &mut Cache, key: Key, image: Arc<RenderImage>, source: Weak<RenderImage>, clock: u64) {
    let size = image.size(0);
    let bytes = (size.width.0.max(0) as usize) * (size.height.0.max(0) as usize) * 4;
    c.bytes += bytes;
    if let Some(old) = c.copies.insert(key, Copy { image, source, bytes, used: clock }) {
        c.bytes -= old.bytes;
        c.retired.push(old.image);
    }
}

fn evict(c: &mut Cache, key: Key) {
    if let Some(copy) = c.copies.remove(&key) {
        c.bytes -= copy.bytes;
        c.retired.push(copy.image);
    }
}

/// A copy of `image`'s `frame` at `target`, or `None` if it has no such frame.
/// Every image in the atlas is straight alpha: gpui's decoders hand it over so,
/// and a `<canvas>` bitmap is un-premultiplied on its way in.
fn make(image: &RenderImage, frame: usize, target: (u32, u32)) -> Option<RenderImage> {
    let size = image.size(frame);
    let bytes = image.as_bytes(frame)?;
    let out = downscale(bytes, size.width.0 as u32, size.height.0 as u32, target.0, target.1, false);
    let buffer = image::RgbaImage::from_raw(target.0, target.1, out)?;
    // The picture's size rides along, so the copy can stand in for the
    // decode ([`reduced_copy`]).
    Some(RenderImage::new(vec![image::Frame::new(buffer)]).with_natural_size(image.natural_size(frame)))
}

fn submit(job: Job) {
    let jobs = JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("vn-resample".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let made = make(&job.source, job.key.1, job.target);
                    let landed = (job.key, Arc::downgrade(&job.source), made);
                    drop(job.source);
                    DONE.lock().unwrap_or_else(|e| e.into_inner()).push(landed);
                }
            })
            .expect("spawn the resampling thread");
        Mutex::new(tx)
    });
    let _ = jobs.lock().unwrap_or_else(|e| e.into_inner()).send(job);
}

/// Once per frame group, from `crate::frame_tick`: installs the resampler,
/// renews the frame's [`SYNC_BUDGET`], takes in the copies that landed,
/// starts the ones whose size has held,
/// evicts copies whose image is gone and the least recently drawn past the
/// budget, and hands the evicted ones back to gpui. `true` when a copy landed,
/// so the window repaints and draws it.
pub(crate) fn tick(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    if enabled() {
        gpui::set_image_resampler(resampler);
    }
    let landed: Vec<Landed> = std::mem::take(&mut *DONE.lock().unwrap_or_else(|e| e.into_inner()));
    let now = Instant::now();
    let (moved, retired) = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.sync_spent = Duration::ZERO;
        let mut moved = false;
        for (key, source, made) in landed {
            c.in_flight.remove(&key);
            if let Some(made) = made.filter(|_| source.strong_count() > 0) {
                let clock = c.clock;
                insert(&mut c, key, Arc::new(made), source, clock);
                moved = true;
            }
        }
        let due: Vec<Key> = c
            .waiting
            .iter()
            .filter(|(_, (source, since))| source.strong_count() == 0 || now.duration_since(*since) >= SETTLE)
            .map(|(k, _)| *k)
            .collect();
        for key in due {
            let Some((source, _)) = c.waiting.remove(&key) else { continue };
            let Some(source) = source.upgrade() else { continue };
            if c.copies.contains_key(&key) || c.in_flight.contains(&key) {
                continue;
            }
            let natural = source.size(key.1);
            let target = (stepped(natural.width.0 as u32, key.2), stepped(natural.height.0 as u32, key.3));
            c.in_flight.insert(key);
            submit(Job { key, source, target });
        }
        let gone: Vec<Key> = c.copies.iter().filter(|(_, v)| v.source.strong_count() == 0).map(|(k, _)| *k).collect();
        for key in gone {
            evict(&mut c, key);
        }
        while c.bytes > BUDGET_BYTES {
            let Some(oldest) = c.copies.iter().min_by_key(|(_, v)| v.used).map(|(k, _)| *k) else { break };
            evict(&mut c, oldest);
        }
        (moved, std::mem::take(&mut c.retired))
    });
    HOLD_REPLAYS.with(|h| h.set(moved || !retired.is_empty()));
    for image in retired {
        cx.drop_image(image, Some(window));
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-pixel black/white checkerboard, opaque.
    fn checker(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let c = if (x + y) % 2 == 0 { 255 } else { 0 };
                v.extend_from_slice(&[c, c, c, 255]);
            }
        }
        v
    }

    fn image(bytes: Vec<u8>, w: u32, h: u32) -> Arc<RenderImage> {
        Arc::new(RenderImage::new(vec![image::Frame::new(image::RgbaImage::from_raw(w, h, bytes).unwrap())]))
    }

    fn device(w: i32, h: i32) -> Size<DevicePixels> {
        Size { width: DevicePixels(w), height: DevicePixels(h) }
    }

    #[test]
    fn steps_round_up_by_a_quarter_octave_and_leave_near_sizes_alone() {
        assert_eq!(step(1.0), 0);
        assert_eq!(step(2.0), 0);
        assert_eq!(step(0.8), 0);
        assert_eq!(step(0.79), -1);
        assert_eq!(step(0.5), -4);
        assert_eq!(step(40.0 / 1080.0), -19);
        // A step's size is never below the drawn one, nor ~19 % above it.
        for drawn in [40u32, 41, 57, 80, 120, 333] {
            let s = stepped(1080, step(drawn as f32 / 1080.0));
            assert!(s >= drawn && s as f32 <= drawn as f32 * 1.19 + 1.0, "{drawn} -> {s}");
        }
        assert_eq!(stepped(1080, 0), 1080);
        assert_eq!(stepped(3, -40), 1);
    }

    /// A new frame's [`SYNC_BUDGET`], as `tick` renews it: a debug build or a
    /// busy machine can spend the whole of it on one copy.
    fn new_frame() {
        CACHE.with(|c| c.borrow_mut().sync_spent = Duration::ZERO);
    }

    #[test]
    fn the_first_copy_is_made_at_once_and_reused() {
        new_frame();
        let img = image(checker(200, 200), 200, 200);
        assert!(resampler(&img, 0, device(200, 200)).is_none());
        assert!(resampler(&img, 0, device(180, 180)).is_none(), "0.9 of its size: drawn as it is");
        assert!(resampler(&img, 0, device(400, 400)).is_none(), "drawn larger: as it is");
        let first = resampler(&img, 0, device(40, 40)).expect("a small image's first copy is made on the spot");
        let size = first.size(0);
        assert!(size.width.0 >= 40 && size.width.0 <= 48, "{size:?}");
        assert!(Arc::ptr_eq(&first, &resampler(&img, 0, device(41, 41)).unwrap()), "same step, same copy");

        // Another size draws the nearest copy and waits.
        let other = resampler(&img, 0, device(100, 100)).unwrap();
        assert!(Arc::ptr_eq(&first, &other));
        let key = (img.id, 0, step(0.5), step(0.5));
        assert!(CACHE.with(|c| c.borrow().waiting.contains_key(&key)));
        // Asking for both sizes, frame after frame, does not restart the wait.
        let since = CACHE.with(|c| c.borrow().waiting[&key].1);
        let _ = resampler(&img, 0, device(40, 40));
        let _ = resampler(&img, 0, device(100, 100));
        assert_eq!(CACHE.with(|c| c.borrow().waiting[&key].1), since);

        // Only one axis small: only that axis is resampled.
        new_frame();
        let wide = image(checker(400, 2), 400, 2);
        let copy = resampler(&wide, 0, device(40, 2)).unwrap();
        assert_eq!((copy.size(0).height.0, copy.size(0).width.0 >= 40), (2, true));

        // An image whose owner is done with it takes its copies along.
        forget(&img);
        assert!(CACHE.with(|c| c.borrow().copies.keys().all(|k| k.0 != img.id)));
        assert!(CACHE.with(|c| c.borrow().retired.iter().any(|r| Arc::ptr_eq(r, &first))));
    }

    #[test]
    fn past_the_frames_budget_a_first_copy_is_made_off_the_painting_thread() {
        CACHE.with(|c| c.borrow_mut().sync_spent = SYNC_BUDGET);
        let img = image(checker(200, 200), 200, 200);
        assert!(resampler(&img, 0, device(40, 40)).is_none());
        assert!(CACHE.with(|c| c.borrow().in_flight.iter().any(|k| k.0 == img.id)));
    }

    #[test]
    fn a_large_image_is_resampled_off_the_painting_thread() {
        let img = image(checker(1200, 1200), 1200, 1200);
        assert!(resampler(&img, 0, device(60, 60)).is_none(), "drawn as it is until the copy lands");
        assert!(resampler(&img, 0, device(60, 60)).is_none(), "and asked for once");
        assert_eq!(CACHE.with(|c| c.borrow().in_flight.len()), 1);
        let deadline = Instant::now() + Duration::from_secs(20);
        let landed = loop {
            let mut done = DONE.lock().unwrap();
            if let Some(i) = done.iter().position(|l| l.0 .0 == img.id) {
                break done.remove(i);
            }
            drop(done);
            assert!(Instant::now() < deadline, "the copy never landed");
            std::thread::sleep(Duration::from_millis(5));
        };
        let (key, source, made) = landed;
        let made = made.expect("a copy");
        assert_eq!(made.size(0).width.0, stepped(1200, step(0.05)) as i32);
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            c.in_flight.remove(&key);
            insert(&mut c, key, Arc::new(made), source, 0);
        });
        assert!(resampler(&img, 0, device(60, 60)).is_some());
    }

    /// Past [`REDUCE_AFTER`], seen from a draw made just now.
    fn settled() -> Instant {
        Instant::now() + REDUCE_AFTER + Duration::from_millis(50)
    }

    #[test]
    fn a_copy_stands_in_for_a_decode_drawn_only_small() {
        new_frame();
        let img = image(checker(600, 600), 600, 600);
        let copy = resampler(&img, 0, device(40, 40)).expect("the first copy, on the spot");
        assert_eq!(copy.natural_size(0), img.size(0), "a copy carries the picture's size");
        assert_eq!(reduced_copy(&img, Instant::now()).err(), Some(KeepFull::Young), "drawn for less than REDUCE_AFTER: the decode stays");

        let reduced = reduced_copy(&img, settled()).expect("drawn only at 40 px: the copy stands in");
        assert!(Arc::ptr_eq(&reduced, &copy));
        assert!(reduced.is_reduced());
        // Out of the copy cache and NOT retired: its atlas tile is what is
        // drawn from next.
        CACHE.with(|c| {
            let c = c.borrow();
            assert!(c.copies.values().all(|v| !Arc::ptr_eq(&v.image, &reduced)));
            assert!(c.retired.iter().all(|r| !Arc::ptr_eq(r, &reduced)));
        });

        // Drawn at about its own size: as it is, wanting nothing.
        let _ = take_wanted_full();
        assert!(resampler(&reduced, 0, device(40, 40)).is_none());
        assert!(take_wanted_full().is_empty());
        // A reduced decode is never reduced again.
        assert_eq!(reduced_copy(&reduced, settled()).err(), Some(KeepFull::NotStill));
        // Drawn large: the full decode is wanted back.
        let _ = resampler(&reduced, 0, device(300, 300));
        assert!(take_wanted_full().contains(&reduced.id));
    }

    #[test]
    fn a_decode_stays_when_a_copy_would_save_little() {
        new_frame();
        // Drawn large as well as small: no copy covers the large draw.
        let big = image(checker(600, 600), 600, 600);
        let _ = resampler(&big, 0, device(40, 40));
        let _ = resampler(&big, 0, device(400, 400));
        assert_eq!(reduced_copy(&big, settled()).err(), Some(KeepFull::NoCopy));
        // A small image: a copy would free less than MIN_SAVING.
        new_frame();
        let small = image(checker(120, 120), 120, 120);
        assert!(resampler(&small, 0, device(20, 20)).is_some());
        assert_eq!(reduced_copy(&small, settled()).err(), Some(KeepFull::SmallSaving));
        // Never drawn: nothing says what size it is needed at.
        let unseen = image(checker(600, 600), 600, 600);
        assert_eq!(reduced_copy(&unseen, settled()).err(), Some(KeepFull::Undrawn));
    }
}
