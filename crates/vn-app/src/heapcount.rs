//! Live Rust heap bytes — the devtools `mem` command's "Rust heap" row (m26).
//!
//! The process's private bytes are several allocators' worth of memory: the
//! Rust heap (this process's system heap, which the static CRT's `malloc`
//! shares), JSC's mimalloc, JSC's executable memory, D3D's driver
//! allocations, libwebrtc once a call starts. Only the first can be counted
//! from Rust, and it is the one the retained tree, the cascade, gpui's element
//! arena and every decoded image live in — so it is counted here, and the
//! rest is attributed by subtraction in `devtools::mem`.
//!
//! ## When it counts
//!
//! The wrapper is the global allocator of every build that has the devtools
//! (`cfg(vn_devtools)`). It COUNTS by default in a dev build and does not in a
//! `--features prod` one; `VN_MEMSTATS` overrides either way (`0` off,
//! anything else on). Off, each allocation pays one relaxed load more than
//! `System` alone. On, it pays six relaxed atomic adds and one `fetch_max` —
//! measured in research/reports/m26/perf-mem-1.md (idle frame and a cold
//! "open a space" trace, on vs off) before it was made the dev default.
//!
//! The switch is read on the very first allocation, before `main`, through
//! `GetEnvironmentVariableW` into a stack buffer — reading it with `std::env`
//! would allocate inside the allocator — so the count starts at the first
//! byte and a free can never be of something that was not counted.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicU8, Ordering::Relaxed};

pub struct Counting;

/// 0 = not looked at yet, 1 = off, 2 = on.
static STATE: AtomicU8 = AtomicU8::new(0);
static LIVE_BYTES: AtomicI64 = AtomicI64::new(0);
static LIVE_BLOCKS: AtomicI64 = AtomicI64::new(0);
static PEAK_BYTES: AtomicI64 = AtomicI64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);

/// Live bytes and blocks per power-of-two size class: class `k` holds blocks
/// of `2^(k-1)..2^k` bytes, the last class everything from `2^(CLASSES-2)` up.
/// Which sizes hold the heap is the first thing "what are these 200 MB" asks:
/// a few 10-MB pixel buffers and a million 64-byte nodes are different bugs.
const CLASSES: usize = 34;
#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicI64 = AtomicI64::new(0);
static CLASS_BYTES: [AtomicI64; CLASSES] = [ZERO; CLASSES];
static CLASS_BLOCKS: [AtomicI64; CLASSES] = [ZERO; CLASSES];

/// The default when `VN_MEMSTATS` is unset: on in a dev build, off in a
/// shipped one.
const DEFAULT_ON: bool = !cfg!(feature = "prod");

#[link(name = "kernel32")]
extern "system" {
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
}

#[cold]
fn probe() -> bool {
    // "VN_MEMSTATS\0", by hand: no allocation may happen here.
    const NAME: [u16; 12] = [
        b'V' as u16, b'N' as u16, b'_' as u16, b'M' as u16, b'E' as u16, b'M' as u16, b'S' as u16,
        b'T' as u16, b'A' as u16, b'T' as u16, b'S' as u16, 0,
    ];
    let mut buf = [0u16; 8];
    let n = unsafe { GetEnvironmentVariableW(NAME.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    // Unset or empty: the build's default. "0": off. Anything else: on.
    let on = if n == 0 { DEFAULT_ON } else { !(n == 1 && buf[0] == b'0' as u16) };
    STATE.store(if on { 2 } else { 1 }, Relaxed);
    on
}

#[inline]
fn counting() -> bool {
    match STATE.load(Relaxed) {
        2 => true,
        1 => false,
        _ => probe(),
    }
}

#[inline]
fn class(size: usize) -> usize {
    ((usize::BITS - size.leading_zeros()) as usize).min(CLASSES - 1)
}

#[inline]
fn add(size: usize, sign: i64) {
    let bytes = sign * size as i64;
    let now = LIVE_BYTES.fetch_add(bytes, Relaxed) + bytes;
    LIVE_BLOCKS.fetch_add(sign, Relaxed);
    if sign > 0 {
        PEAK_BYTES.fetch_max(now, Relaxed);
    }
    let k = class(size);
    CLASS_BYTES[k].fetch_add(bytes, Relaxed);
    CLASS_BLOCKS[k].fetch_add(sign, Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() && counting() {
            add(layout.size(), 1);
            ALLOCS.fetch_add(1, Relaxed);
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() && counting() {
            add(layout.size(), 1);
            ALLOCS.fetch_add(1, Relaxed);
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        if counting() {
            add(layout.size(), -1);
        }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() && counting() {
            add(layout.size(), -1);
            add(new_size, 1);
        }
        p
    }
}

/// `Some((live bytes, live blocks, peak bytes, allocations since start))`
/// while counting, `None` when it is off.
pub fn snapshot() -> Option<(i64, i64, i64, u64)> {
    on().then(|| {
        (LIVE_BYTES.load(Relaxed), LIVE_BLOCKS.load(Relaxed), PEAK_BYTES.load(Relaxed), ALLOCS.load(Relaxed))
    })
}

/// `(upper bound in bytes, live bytes, live blocks)` per non-empty size class,
/// smallest first. The last class has no upper bound (`u64::MAX`).
pub fn classes() -> Vec<(u64, i64, i64)> {
    (0..CLASSES)
        .filter_map(|k| {
            let blocks = CLASS_BLOCKS[k].load(Relaxed);
            let bytes = CLASS_BYTES[k].load(Relaxed);
            let upper = if k == CLASSES - 1 { u64::MAX } else { 1u64 << k };
            (blocks != 0).then_some((upper, bytes, blocks))
        })
        .collect()
}

/// Is the Rust heap being counted? Only when [`Counting`] IS the global
/// allocator — the exe's is (src/main.rs), an embedder's need not be — and
/// then it has counted the first allocation long before anyone asks.
pub fn on() -> bool {
    counting() && ALLOCS.load(Relaxed) > 0
}
