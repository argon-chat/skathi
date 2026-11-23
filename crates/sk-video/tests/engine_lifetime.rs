//! A `<video>` that goes away takes its Media Foundation pipeline with it.
//!
//! Releasing an `IMFMediaEngine` without `Shutdown()` leaves its pipeline (source, decoders,
//! work-queue threads) alive. This mounts and drops a clip many times and checks that the threads
//! the process gains do not grow with the number of elements.

#![cfg(windows)]

use std::collections::BTreeMap;
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
    fn OpenThread(access: u32, inherit: i32, id: u32) -> isize;
    fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut isize) -> i32;
    fn GetModuleFileNameW(module: isize, name: *mut u16, size: u32) -> u32;
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationThread(
        thread: isize,
        class: u32,
        info: *mut usize,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}

/// The module a thread started in (`ntdll.dll` for the OS thread pool's workers).
fn start_module(thread_id: u32) -> String {
    const THREAD_QUERY_INFORMATION: u32 = 0x40;
    const THREAD_QUERY_SET_WIN32_START_ADDRESS: u32 = 9;
    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;
    let handle = unsafe { OpenThread(THREAD_QUERY_INFORMATION, 0, thread_id) };
    if handle == 0 {
        return "?".into();
    }
    let mut start = 0usize;
    let status = unsafe {
        NtQueryInformationThread(
            handle,
            THREAD_QUERY_SET_WIN32_START_ADDRESS,
            &mut start,
            std::mem::size_of::<usize>() as u32,
            std::ptr::null_mut(),
        )
    };
    unsafe { CloseHandle(handle) };
    let mut module = 0isize;
    if status < 0
        || unsafe {
            GetModuleHandleExW(
                FROM_ADDRESS | UNCHANGED_REFCOUNT,
                start as *const u16,
                &mut module,
            )
        } == 0
    {
        return "?".into();
    }
    let mut name = [0u16; 260];
    let n = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) };
    let path = String::from_utf16_lossy(&name[..n as usize]);
    path.rsplit('\\')
        .next()
        .unwrap_or(&path)
        .to_ascii_lowercase()
}

/// The process's threads, counted by the module each started in.
fn threads() -> BTreeMap<String, usize> {
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
    let mut by_module = BTreeMap::new();
    let mut ok = unsafe { Thread32First(snap, &mut e) };
    while ok != 0 {
        if e.owner_process_id == me {
            *by_module.entry(start_module(e.thread_id)).or_insert(0) += 1;
        }
        ok = unsafe { Thread32Next(snap, &mut e) };
    }
    unsafe { CloseHandle(snap) };
    by_module
}

fn total(by_module: &BTreeMap<String, usize>) -> usize {
    by_module.values().sum()
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

/// The thread census once the count has stopped falling for a while, or after `ms`.
fn settled_threads(ms: u64) -> BTreeMap<String, usize> {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut last = threads();
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
        let now = threads();
        if total(&now) >= total(&last) && Instant::now() + Duration::from_millis(300) >= until {
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
    const CYCLES: usize = 4;
    for _ in 0..CYCLES {
        assert!(one_element(&clip));
    }
    let after_few = settled_threads(3000);
    for _ in 0..2 * CYCLES {
        assert!(one_element(&clip));
    }
    let after_many = settled_threads(3000);
    eprintln!("threads by start module: {baseline:?} after the first element");
    eprintln!(
        "  {after_few:?} after {CYCLES} more, {after_many:?} after {} more",
        2 * CYCLES
    );
    // Thread pools grow and shrink with load by themselves; a pipeline left behind per element
    // adds at least one thread for each of the last 2 * CYCLES elements.
    let (few, many) = (total(&after_few), total(&after_many));
    assert!(
        many < few + CYCLES,
        "{} more elements took the process from {few} to {many} threads",
        2 * CYCLES
    );
}
