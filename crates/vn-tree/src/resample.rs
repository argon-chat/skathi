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
//! **Divergence (IMG-1, docs/RUNTIME.md §5.16):** the size is the box's, not
//! the one a CSS `transform: scale()` gives it on screen.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use gpui::{DevicePixels, ImageId, RenderImage, Size};

/// Below this fraction of its size along an axis, an image is drawn from a copy.
const RESAMPLE_BELOW: f32 = 0.8;
/// Size steps per octave.
const STEPS: f32 = 4.0;
/// The largest image (in source pixels) whose first copy is made on the
/// painting thread: a 1080 × 1080 avatar is 1.17 M, and its copy takes about
/// 1.5 ms in a release build (`tests::an_avatar_downscales_quickly`).
const SYNC_PIXELS: u64 = 1_300_000;
/// What one frame may spend making first copies on the painting thread. A
/// list of new avatars past it gets the rest of its copies from the
/// resampling thread a frame or two later, instead of one long frame.
const SYNC_BUDGET: Duration = Duration::from_millis(4);
/// How long another size of an image must be asked for before it gets a copy.
const SETTLE: Duration = Duration::from_millis(100);
/// The bytes of copies kept.
const BUDGET_BYTES: usize = 64 << 20;

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
    /// Images whose pixels are premultiplied (a `<canvas>` bitmap); the rest
    /// are straight alpha, as gpui decodes them.
    premultiplied: HashSet<ImageId>,
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
}

/// One copy to make on the resampling thread.
struct Job {
    key: Key,
    source: Arc<RenderImage>,
    target: (u32, u32),
    premultiplied: bool,
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

/// Marks a `<canvas>` bitmap as premultiplied, so its copies are filtered as
/// such. Undone by [`forget`].
pub(crate) fn mark_premultiplied(image: &RenderImage) {
    CACHE.with(|c| c.borrow_mut().premultiplied.insert(image.id));
}

/// An image its owner is done with: its copies go back to gpui at the next tick.
pub(crate) fn forget(image: &RenderImage) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.premultiplied.remove(&image.id);
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
                let premultiplied = c.premultiplied.contains(&image.id);
                if (nw as u64) * (nh as u64) <= SYNC_PIXELS && c.sync_spent < SYNC_BUDGET {
                    let t = Instant::now();
                    let made = Arc::new(make(image, frame, target, premultiplied)?);
                    c.sync_spent += t.elapsed();
                    insert(&mut c, key, made.clone(), Arc::downgrade(image), clock);
                    Some(made)
                } else {
                    c.in_flight.insert(key);
                    submit(Job { key, source: image.clone(), target, premultiplied });
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
fn make(image: &RenderImage, frame: usize, target: (u32, u32), premultiplied: bool) -> Option<RenderImage> {
    let size = image.size(frame);
    let bytes = image.as_bytes(frame)?;
    let out = downscale(bytes, size.width.0 as u32, size.height.0 as u32, target.0, target.1, premultiplied);
    let buffer = image::RgbaImage::from_raw(target.0, target.1, out)?;
    Some(RenderImage::new(vec![image::Frame::new(buffer)]))
}

fn submit(job: Job) {
    let jobs = JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("vn-resample".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let made = make(&job.source, job.key.1, job.target, job.premultiplied);
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
            let premultiplied = c.premultiplied.contains(&key.0);
            c.in_flight.insert(key);
            submit(Job { key, source, target, premultiplied });
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

/// `w` × `h` pixels of 4 bytes, BGRA or RGBA (the channel order does not
/// matter here), filtered down to `tw` × `th`. Straight alpha unless
/// `premultiplied`; the result keeps the input's convention.
pub(crate) fn downscale(bytes: &[u8], w: u32, h: u32, tw: u32, th: u32, premultiplied: bool) -> Vec<u8> {
    use image::{imageops, ImageBuffer, Rgba};
    let (tw, th) = (tw.clamp(1, w.max(1)), th.clamp(1, h.max(1)));
    // First a k×k box average, k the largest power of two that leaves the
    // image at least twice the target (2×2 box steps, as a mipmap chain makes
    // them, done in one pass), so Lanczos has a ratio under 4 left to cover.
    // In integers straight off the bytes, premultiplied, so the filter weighs
    // colour by coverage: this pass reads every source pixel, and it is most of
    // the cost.
    let mut k = 1u32;
    while w / (k * 2) >= tw * 2 && h / (k * 2) >= th * 2 && k < 128 {
        k *= 2;
    }
    let (bw, bh) = (w / k, h / k);
    let mut px = vec![0f32; (bw * bh * 4) as usize];
    // Straight alpha sums c·a (up to 255² a pixel), premultiplied sums c.
    let alpha_scale = 1.0 / (255.0 * (k * k) as f32);
    let colour_scale = if premultiplied { alpha_scale } else { alpha_scale / 255.0 };
    let mut acc = vec![[0u32; 4]; bw as usize];
    for by in 0..bh {
        acc.iter_mut().for_each(|a| *a = [0; 4]);
        for y in by * k..(by + 1) * k {
            let row = &bytes[(y * w * 4) as usize..((y * w + bw * k) * 4) as usize];
            if premultiplied {
                box_row::<true>(row, k, &mut acc);
            } else {
                box_row::<false>(row, k, &mut acc);
            }
        }
        let out = &mut px[(by * bw * 4) as usize..((by + 1) * bw * 4) as usize];
        for (o, s) in out.chunks_exact_mut(4).zip(&acc) {
            o[0] = s[0] as f32 * colour_scale;
            o[1] = s[1] as f32 * colour_scale;
            o[2] = s[2] as f32 * colour_scale;
            o[3] = s[3] as f32 * alpha_scale;
        }
    }
    let (w, h) = (bw, bh);
    let buffer: ImageBuffer<Rgba<f32>, Vec<f32>> = ImageBuffer::from_raw(w, h, px).expect("w * h * 4 floats");
    let small = if (w, h) == (tw, th) { buffer } else { imageops::resize(&buffer, tw, th, imageops::FilterType::Lanczos3) };
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for p in small.pixels() {
        let a = p[3].clamp(0.0, 1.0);
        let k = if premultiplied {
            255.0
        } else if a > 0.0 {
            255.0 / a
        } else {
            0.0
        };
        let c = |v: f32| (v.clamp(0.0, a) * k).round() as u8;
        out.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), (a * 255.0).round() as u8]);
    }
    out
}

/// One source row into the per-block sums of [`downscale`]'s box pass: `k`
/// pixels a block, colour weighted by alpha unless `PREMULTIPLIED`.
#[inline(always)]
fn box_row<const PREMULTIPLIED: bool>(row: &[u8], k: u32, acc: &mut [[u32; 4]]) {
    for (block, s) in row.chunks_exact((k * 4) as usize).zip(acc.iter_mut()) {
        let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
        for p in block.chunks_exact(4) {
            let pa = p[3] as u32;
            let w = if PREMULTIPLIED { 1 } else { pa };
            r += p[0] as u32 * w;
            g += p[1] as u32 * w;
            b += p[2] as u32 * w;
            a += pa;
        }
        s[0] += r;
        s[1] += g;
        s[2] += b;
        s[3] += a;
    }
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
    fn a_checkerboard_averages_to_grey_instead_of_aliasing() {
        let out = downscale(&checker(1080, 1080), 1080, 1080, 40, 40, false);
        assert_eq!(out.len(), 40 * 40 * 4);
        for p in out.chunks_exact(4) {
            assert!((p[0] as i32 - 128).abs() <= 2, "{p:?}");
            assert_eq!(p[3], 255);
        }
        // An odd ratio, where the box steps stop early and Lanczos does more.
        let out = downscale(&checker(1080, 1080), 1080, 1080, 47, 47, false);
        for p in out.chunks_exact(4) {
            assert!((p[0] as i32 - 128).abs() <= 3, "{p:?}");
        }
    }

    #[test]
    fn a_transparent_pixel_adds_no_colour() {
        // Left half opaque red, right half fully transparent GREEN: a filter
        // that ignores alpha bleeds green into the edge.
        let (w, h) = (64u32, 8u32);
        let mut v = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                v.extend_from_slice(if x < w / 2 { &[255, 0, 0, 255] } else { &[0, 255, 0, 0] });
            }
        }
        let out = downscale(&v, w, h, 8, 1, false);
        for p in out.chunks_exact(4) {
            if p[3] > 0 {
                assert!(p[1] <= 2 && p[0] >= 250, "edge pixel {p:?}");
            }
        }
        // Premultiplied in, premultiplied out: colour never above alpha.
        let pre: Vec<u8> = v.iter().enumerate().map(|(i, b)| if i % 4 == 1 { 0 } else { *b }).collect();
        for p in downscale(&pre, w, h, 8, 1, true).chunks_exact(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "{p:?}");
        }
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

    #[test]
    fn an_avatar_downscales_quickly() {
        let src = checker(1080, 1080);
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            let _ = std::hint::black_box(downscale(&src, 1080, 1080, 80, 80, false));
            best = best.min(t.elapsed());
        }
        eprintln!("1080x1080 -> 80x80: {best:?}");
        // A debug build is far slower; the budget only means something in release.
        if !cfg!(debug_assertions) {
            assert!(best < Duration::from_millis(8), "{best:?}");
        }
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
}
