//! A `<video>` that goes away takes its Media Foundation pipeline with it.
//!
//! Releasing an `IMFMediaEngine` without `Shutdown()` leaves its pipeline (source, decoders,
//! work-queue threads) alive. This mounts and drops a clip several times and counts the process's threads.

#![cfg(windows)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use sk_video::{FrameRequest, MediaSource, ReadyState, VideoState};

#[repr(C)]
struct ThreadEntry32 {
    size: u32,
    usage: u32,
    thread_id: u32,
    owner_process_id: u32,
    base_priority: i32,
    delta_priority: i32,
    flags: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, process: u32) -> isize;
    fn Thread32First(snapshot: isize, entry: *mut ThreadEntry32) -> i32;
    fn Thread32Next(snapshot: isize, entry: *mut ThreadEntry32) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn GetCurrentProcessId() -> u32;
}

fn threads() -> usize {
    const TH32CS_SNAPTHREAD: u32 = 0x4;
    let me = unsafe { GetCurrentProcessId() };
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    assert!(snap != -1, "CreateToolhelp32Snapshot failed");
    let mut e = ThreadEntry32 {
        size: std::mem::size_of::<ThreadEntry32>() as u32,
        usage: 0,
        thread_id: 0,
        owner_process_id: 0,
        base_priority: 0,
        delta_priority: 0,
        flags: 0,
    };
    let mut n = 0;
    let mut ok = unsafe { Thread32First(snap, &mut e) };
    while ok != 0 {
        if e.owner_process_id == me {
            n += 1;
        }
        ok = unsafe { Thread32Next(snap, &mut e) };
    }
    unsafe { CloseHandle(snap) };
    n
}

/// Mounts a `<video>` on the clip, plays it until it has a frame (or 5 s), and drops it; `false` when there is no media backend.
fn one_element(clip: &PathBuf) -> bool {
    let Ok(backend) = sk_video::backend_for_this_platform() else {
        return false;
    };
    let mut state = VideoState::new(backend);
    state
        .set_source(MediaSource::Path(clip.clone()))
        .expect("set_source");
    state.set_muted(true);
    state.play();
    let request = FrameRequest { max_size: (64, 64) };
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let advance = state.advance(Instant::now(), &request);
        let _ = state.take_retired();
        if advance.new_frame || state.ready_state() >= ReadyState::HaveCurrentData {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let _ = state.take_all_images();
    true
}

fn settled_threads(ms: u64) -> usize {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut last = threads();
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
        let now = threads();
        if now >= last && Instant::now() + Duration::from_millis(300) >= until {
            return now;
        }
        last = now;
    }
    last
}

#[test]
fn an_element_that_goes_leaves_no_pipeline_behind() {
    let clip =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/hello/assets/clip-tiny.mp4");
    let clip = clip
        .canonicalize()
        .expect("apps/hello/assets/clip-tiny.mp4");
    // The first element starts Media Foundation's shared work queues, which live for the process.
    if !one_element(&clip) {
        eprintln!("no media backend on this machine: skipped");
        return;
    }
    let baseline = settled_threads(1500);
    const CYCLES: usize = 6;
    for _ in 0..CYCLES {
        assert!(one_element(&clip));
    }
    let after = settled_threads(3000);
    eprintln!("threads: {baseline} after the first element, {after} after {CYCLES} more");
    assert!(
        after <= baseline + 3,
        "{CYCLES} elements mounted and dropped took the process from {baseline} to {after} threads"
    );
}
