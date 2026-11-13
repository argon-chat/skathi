//! Live Rust heap bytes: the devtools `mem` command's "Rust heap" row.
//!
//! The process's private bytes span several allocators (the Rust heap, which the static CRT's
//! `malloc` shares; JSC's mimalloc and executable memory; D3D's driver allocations; libwebrtc). Only
//! the Rust heap can be counted from Rust, and the retained tree, cascade, gpui's arena and every
//! decoded image live in it, so it is counted here and the rest is attributed by subtraction in
//! `devtools::mem`.
//!
//! The wrapper is the global allocator of every build that has the devtools (`cfg(vn_devtools)`). It
//! counts by default in a dev build and not in a `--features prod` one; `VN_MEMSTATS` overrides
//! either way (`0` off, anything else on).
//!
//! The switch is read on the very first allocation, before `main`, through `GetEnvironmentVariableW`
//! into a stack buffer (`std::env` would allocate inside the allocator), so the count starts at the
//! first byte and a free can never be of something that was not counted.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, AtomicU8, AtomicU64, Ordering::Relaxed};

pub struct Counting;

static STATE: AtomicU8 = AtomicU8::new(0);
static LIVE_BYTES: AtomicI64 = AtomicI64::new(0);
static LIVE_BLOCKS: AtomicI64 = AtomicI64::new(0);
static PEAK_BYTES: AtomicI64 = AtomicI64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);

const CLASSES: usize = 34;
#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicI64 = AtomicI64::new(0);
static CLASS_BYTES: [AtomicI64; CLASSES] = [ZERO; CLASSES];
static CLASS_BLOCKS: [AtomicI64; CLASSES] = [ZERO; CLASSES];

const DEFAULT_ON: bool = !cfg!(feature = "prod");

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
}

#[cfg(windows)]
fn memstats_env() -> Option<bool> {
    const NAME: [u16; 12] = [
        b'V' as u16,
        b'N' as u16,
        b'_' as u16,
        b'M' as u16,
        b'E' as u16,
        b'M' as u16,
        b'S' as u16,
        b'T' as u16,
        b'A' as u16,
        b'T' as u16,
        b'S' as u16,
        0,
    ];
    let mut buf = [0u16; 8];
    let n = unsafe { GetEnvironmentVariableW(NAME.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    (n != 0).then(|| !(n == 1 && buf[0] == b'0' as u16))
}

#[cfg(not(windows))]
fn memstats_env() -> Option<bool> {
    extern "C" {
        fn getenv(name: *const core::ffi::c_char) -> *const core::ffi::c_char;
    }
    // `getenv` returns the environment's own storage: no copy, no allocation.
    let value = unsafe { getenv(c"VN_MEMSTATS".as_ptr()) };
    if value.is_null() {
        return None;
    }
    let first = unsafe { *value } as u8;
    if first == 0 {
        return None;
    }
    let second = unsafe { *value.add(1) } as u8;
    Some(!(first == b'0' && second == 0))
}

#[cold]
fn probe() -> bool {
    let on = memstats_env().unwrap_or(DEFAULT_ON);
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
        if !p.is_null() {
            recording::allocated(p as usize, layout.size());
        }
        if !p.is_null() && counting() {
            add(layout.size(), 1);
            ALLOCS.fetch_add(1, Relaxed);
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            recording::allocated(p as usize, layout.size());
        }
        if !p.is_null() && counting() {
            add(layout.size(), 1);
            ALLOCS.fetch_add(1, Relaxed);
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        recording::freed(ptr as usize, layout.size());
        unsafe { System.dealloc(ptr, layout) };
        if counting() {
            add(layout.size(), -1);
        }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let ticket = recording::begin_realloc();
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        recording::finish_realloc(ticket, ptr as usize, layout.size(), p as usize, new_size);
        if !p.is_null() && counting() {
            add(layout.size(), -1);
            add(new_size, 1);
        }
        p
    }
}

pub fn snapshot() -> Option<(i64, i64, i64, u64)> {
    on().then(|| {
        (
            LIVE_BYTES.load(Relaxed),
            LIVE_BLOCKS.load(Relaxed),
            PEAK_BYTES.load(Relaxed),
            ALLOCS.load(Relaxed),
        )
    })
}

pub fn classes() -> Vec<(u64, i64, i64)> {
    (0..CLASSES)
        .filter_map(|k| {
            let blocks = CLASS_BLOCKS[k].load(Relaxed);
            let bytes = CLASS_BYTES[k].load(Relaxed);
            let upper = if k == CLASSES - 1 {
                u64::MAX
            } else {
                1u64 << k
            };
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

/// Bounded allocation events. The hook only loads TLS, updates atomics and
/// writes its uniquely reserved slot; it never allocates, locks or unwinds a
/// stack. Session control and site registration happen outside the allocator.
pub mod recording {
    use std::{
        cell::{Cell, UnsafeCell},
        marker::PhantomData,
        rc::Rc,
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
        },
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    pub const MAX_EVENTS: usize = 65_536;
    pub const MAX_SITES: usize = 512;
    static EPOCH: AtomicU64 = AtomicU64::new(0);
    static WRITERS: AtomicUsize = AtomicUsize::new(0);
    static RECORDER: AtomicPtr<Recorder> = AtomicPtr::new(std::ptr::null_mut());
    static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
    static NEXT_THREAD: AtomicUsize = AtomicUsize::new(1);
    thread_local! {
        static SUPPRESSED: Cell<u32> = const { Cell::new(0) };
        static SITE: Cell<u32> = const { Cell::new(0) };
        static THREAD: Cell<u32> = const { Cell::new(0) };
    }

    #[derive(Clone, Copy, Default)]
    pub struct Event {
        pub kind: u8,
        pub address: usize,
        pub size: usize,
        pub new_address: usize,
        pub new_size: usize,
        pub site: u32,
        pub thread: u32,
    }

    struct Slot {
        ready: AtomicBool,
        event: UnsafeCell<Event>,
    }
    // Each slot has one reserved writer; the reader only touches published
    // slots. Reset happens with capture disabled and every writer drained.
    unsafe impl Sync for Slot {}
    struct Recorder {
        slots: Box<[Slot]>,
        next: AtomicUsize,
        dropped: AtomicU64,
        capacity: AtomicUsize,
        sample_every: AtomicUsize,
    }
    struct Session {
        id: u64,
        started: Instant,
        started_ms: f64,
        stopped: Option<Instant>,
    }
    static SESSION: Mutex<Option<Session>> = Mutex::new(None);
    #[derive(Clone)]
    pub struct Site {
        pub category: String,
        pub name: String,
    }
    static SITES: Mutex<Vec<Site>> = Mutex::new(Vec::new());

    pub struct Suppress {
        _thread: PhantomData<Rc<()>>,
    }
    impl Drop for Suppress {
        fn drop(&mut self) {
            let _ = SUPPRESSED.try_with(|value| value.set(value.get().saturating_sub(1)));
        }
    }
    pub fn suppress() -> Suppress {
        let _ = SUPPRESSED.try_with(|value| value.set(value.get().saturating_add(1)));
        Suppress {
            _thread: PhantomData,
        }
    }

    pub struct Scope {
        previous: Option<u32>,
        _thread: PhantomData<Rc<()>>,
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            if let Some(previous) = self.previous {
                let _ = SITE.try_with(|site| site.set(previous));
            }
        }
    }
    #[track_caller]
    pub fn scope(category: &str, name: &str) -> Scope {
        if EPOCH.load(Ordering::Relaxed) == 0 {
            return Scope {
                previous: None,
                _thread: PhantomData,
            };
        }
        let _suppressed = suppress();
        let location = std::panic::Location::caller();
        let label: String = format!("{name} ({}:{})", location.file(), location.line())
            .chars()
            .take(240)
            .collect();
        let category: String = category.chars().take(80).collect();
        let id = {
            let mut sites = SITES.lock().unwrap_or_else(|error| error.into_inner());
            match sites
                .iter()
                .position(|site| site.category == category && site.name == label)
            {
                Some(index) => index as u32 + 1,
                None if sites.len() < MAX_SITES => {
                    sites.push(Site {
                        category,
                        name: label,
                    });
                    sites.len() as u32
                }
                None => 0,
            }
        };
        Scope {
            previous: SITE.try_with(|site| site.replace(id)).ok(),
            _thread: PhantomData,
        }
    }

    #[derive(Clone)]
    pub struct Status {
        pub session: u64,
        pub recording: bool,
        pub started_at_ms: f64,
        pub elapsed_ms: f64,
        pub sample_every: usize,
        pub capacity: usize,
        pub reserved: usize,
        pub dropped: u64,
    }
    fn recorder() -> &'static Recorder {
        static STORAGE: OnceLock<Recorder> = OnceLock::new();
        STORAGE.get_or_init(|| Recorder {
            slots: (0..MAX_EVENTS)
                .map(|_| Slot {
                    ready: AtomicBool::new(false),
                    event: UnsafeCell::new(Event::default()),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            next: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
            capacity: AtomicUsize::new(MAX_EVENTS),
            sample_every: AtomicUsize::new(1),
        })
    }
    fn stop_writers() {
        EPOCH.store(0, Ordering::SeqCst);
        while WRITERS.load(Ordering::SeqCst) != 0 {
            std::thread::yield_now();
        }
    }
    pub fn start(capacity: usize, sample_every: usize) -> Status {
        let _suppressed = suppress();
        let mut session = SESSION.lock().unwrap_or_else(|error| error.into_inner());
        stop_writers();
        let recorder = recorder();
        for slot in recorder.slots.iter() {
            slot.ready.store(false, Ordering::Relaxed);
        }
        recorder.next.store(0, Ordering::Relaxed);
        recorder.dropped.store(0, Ordering::Relaxed);
        recorder
            .capacity
            .store(capacity.clamp(1024, MAX_EVENTS), Ordering::Relaxed);
        recorder
            .sample_every
            .store(sample_every.clamp(1, 4096), Ordering::Relaxed);
        let id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        *session = Some(Session {
            id,
            started: Instant::now(),
            started_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
                * 1000.0,
            stopped: None,
        });
        RECORDER.store(recorder as *const _ as *mut _, Ordering::Release);
        EPOCH.store(id, Ordering::SeqCst);
        drop(session);
        status()
    }
    pub fn stop() -> Status {
        let _suppressed = suppress();
        let mut session = SESSION.lock().unwrap_or_else(|error| error.into_inner());
        stop_writers();
        if let Some(session) = session.as_mut() {
            session.stopped.get_or_insert_with(Instant::now);
        }
        drop(session);
        status()
    }
    pub fn clear() {
        let _suppressed = suppress();
        let mut session = SESSION.lock().unwrap_or_else(|error| error.into_inner());
        stop_writers();
        let pointer = RECORDER.load(Ordering::Acquire);
        if !pointer.is_null() {
            let recorder = unsafe { &*pointer };
            recorder.next.store(0, Ordering::Relaxed);
            recorder.dropped.store(0, Ordering::Relaxed);
        }
        *session = None;
    }
    pub fn reserved_bytes() -> usize {
        if RECORDER.load(Ordering::Acquire).is_null() {
            0
        } else {
            MAX_EVENTS * std::mem::size_of::<Slot>()
        }
    }
    pub fn status() -> Status {
        let _suppressed = suppress();
        let session = SESSION.lock().unwrap_or_else(|error| error.into_inner());
        let pointer = RECORDER.load(Ordering::Acquire);
        let (sample_every, capacity, reserved, dropped) = if pointer.is_null() {
            (1, 0, 0, 0)
        } else {
            let recorder = unsafe { &*pointer };
            (
                recorder.sample_every.load(Ordering::Relaxed),
                recorder.capacity.load(Ordering::Relaxed),
                recorder.next.load(Ordering::Acquire),
                recorder.dropped.load(Ordering::Relaxed),
            )
        };
        Status {
            session: session.as_ref().map_or(0, |session| session.id),
            recording: EPOCH.load(Ordering::Acquire) != 0,
            started_at_ms: session.as_ref().map_or(0.0, |session| session.started_ms),
            elapsed_ms: session.as_ref().map_or(0.0, |session| {
                session
                    .stopped
                    .unwrap_or_else(Instant::now)
                    .duration_since(session.started)
                    .as_secs_f64()
                    * 1000.0
            }),
            sample_every,
            capacity,
            reserved,
            dropped,
        }
    }
    pub fn sites() -> Vec<Site> {
        let _suppressed = suppress();
        SITES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    pub fn drain(cursor: &mut usize, limit: usize) -> Vec<Event> {
        let _suppressed = suppress();
        let _session = SESSION.lock().unwrap_or_else(|error| error.into_inner());
        let pointer = RECORDER.load(Ordering::Acquire);
        if pointer.is_null() {
            return Vec::new();
        }
        let recorder = unsafe { &*pointer };
        let end = recorder
            .next
            .load(Ordering::Acquire)
            .min(recorder.capacity.load(Ordering::Relaxed));
        let mut events = Vec::with_capacity(end.saturating_sub(*cursor).min(limit));
        while *cursor < end && events.len() < limit {
            let slot = &recorder.slots[*cursor];
            if !slot.ready.load(Ordering::Acquire) {
                break;
            }
            events.push(unsafe { *slot.event.get() });
            *cursor += 1;
        }
        events
    }
    pub fn selected(address: usize, sample_every: usize) -> bool {
        let mut hash = address as u64;
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xff51afd7ed558ccd);
        hash ^= hash >> 33;
        hash % sample_every.max(1) as u64 == 0
    }
    pub struct Ticket {
        recorder: &'static Recorder,
        index: usize,
        site: u32,
        thread: u32,
    }
    #[inline]
    fn reserve(address: Option<usize>) -> Option<Ticket> {
        if EPOCH.load(Ordering::Relaxed) == 0 {
            return None;
        }
        if SUPPRESSED
            .try_with(|value| value.get() != 0)
            .unwrap_or(true)
        {
            return None;
        }
        WRITERS.fetch_add(1, Ordering::SeqCst);
        if EPOCH.load(Ordering::SeqCst) == 0 {
            WRITERS.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        let pointer = RECORDER.load(Ordering::Acquire);
        let recorder = unsafe { &*pointer };
        if address.is_some_and(|address| {
            !selected(address, recorder.sample_every.load(Ordering::Relaxed))
        }) {
            WRITERS.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        let index = recorder.next.fetch_add(1, Ordering::AcqRel);
        if index >= recorder.capacity.load(Ordering::Relaxed) {
            recorder.dropped.fetch_add(1, Ordering::Relaxed);
            WRITERS.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        let site = SITE.try_with(Cell::get).unwrap_or(0);
        let thread = THREAD
            .try_with(|thread| {
                let mut id = thread.get();
                if id == 0 {
                    id = NEXT_THREAD.fetch_add(1, Ordering::Relaxed) as u32;
                    thread.set(id);
                }
                id
            })
            .unwrap_or(0);
        Some(Ticket {
            recorder,
            index,
            site,
            thread,
        })
    }
    #[inline]
    fn publish(ticket: Ticket, mut event: Event) {
        event.site = ticket.site;
        event.thread = ticket.thread;
        let slot = &ticket.recorder.slots[ticket.index];
        unsafe {
            *slot.event.get() = event;
        }
        slot.ready.store(true, Ordering::Release);
        WRITERS.fetch_sub(1, Ordering::SeqCst);
    }
    #[inline]
    pub(super) fn allocated(address: usize, size: usize) {
        if let Some(ticket) = reserve(Some(address)) {
            publish(
                ticket,
                Event {
                    kind: 1,
                    address,
                    size,
                    ..Event::default()
                },
            );
        }
    }
    #[inline]
    pub(super) fn freed(address: usize, size: usize) {
        if let Some(ticket) = reserve(Some(address)) {
            publish(
                ticket,
                Event {
                    kind: 2,
                    address,
                    size,
                    ..Event::default()
                },
            );
        }
    }
    #[inline]
    pub(super) fn begin_realloc() -> Option<Ticket> {
        reserve(None)
    }
    #[inline]
    pub(super) fn finish_realloc(
        ticket: Option<Ticket>,
        address: usize,
        size: usize,
        new_address: usize,
        new_size: usize,
    ) {
        if let Some(ticket) = ticket {
            publish(
                ticket,
                Event {
                    kind: if new_address == 0 { 0 } else { 3 },
                    address,
                    size,
                    new_address,
                    new_size,
                    ..Event::default()
                },
            );
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::alloc::{GlobalAlloc, Layout};
        static TEST_SESSION: Mutex<()> = Mutex::new(());
        #[test]
        fn capture_guards_cannot_move_or_share_between_threads() {
            // The second impl makes inference ambiguous if a guard ever
            // acquires Send/Sync. This checks the negative bound without a
            // dependency or a compile-fail test that could fail for other reasons.
            trait NotSend<Marker> {
                fn check() {}
            }
            impl<T: ?Sized> NotSend<()> for T {}
            impl<T: ?Sized + Send> NotSend<u8> for T {}
            trait NotSync<Marker> {
                fn check() {}
            }
            impl<T: ?Sized> NotSync<()> for T {}
            impl<T: ?Sized + Sync> NotSync<u8> for T {}
            let _ = <Scope as NotSend<_>>::check;
            let _ = <Scope as NotSync<_>>::check;
            let _ = <Suppress as NotSend<_>>::check;
            let _ = <Suppress as NotSync<_>>::check;
        }
        #[test]
        fn allocator_records_paired_reallocation_without_recording_profiler_work() {
            let _session = TEST_SESSION.lock().unwrap();
            start(4096, 1);
            let _scope = scope("fixture", "paired realloc");
            let id = SITE.with(Cell::get);
            let layout = Layout::from_size_align(48, 8).unwrap();
            unsafe {
                let original = crate::heapcount::Counting.alloc(layout);
                assert!(!original.is_null());
                let resized = crate::heapcount::Counting.realloc(original, layout, 96);
                assert!(!resized.is_null());
                crate::heapcount::Counting
                    .dealloc(resized, Layout::from_size_align(96, 8).unwrap());
            }
            stop();
            let mut cursor = 0;
            let events: Vec<_> = drain(&mut cursor, MAX_EVENTS)
                .into_iter()
                .filter(|event| event.site == id)
                .collect();
            assert_eq!(
                events.iter().map(|event| event.kind).collect::<Vec<_>>(),
                vec![1, 3, 2]
            );
            assert_eq!(events[0].size, 48);
            assert_eq!(events[1].new_size, 96);
            assert_eq!(events[1].new_address, events[2].address);
            let before = status().reserved;
            unsafe {
                let pointer = crate::heapcount::Counting.alloc(layout);
                crate::heapcount::Counting.dealloc(pointer, layout);
            }
            assert_eq!(
                status().reserved,
                before,
                "disabled recorder retained events"
            );
            clear();
        }
        #[test]
        fn capture_overflow_is_bounded_and_reported() {
            let _session = TEST_SESSION.lock().unwrap();
            start(1024, 1);
            let layout = Layout::from_size_align(24, 8).unwrap();
            for _ in 0..800 {
                unsafe {
                    let pointer = crate::heapcount::Counting.alloc(layout);
                    assert!(!pointer.is_null());
                    crate::heapcount::Counting.dealloc(pointer, layout);
                }
            }
            let status = stop();
            assert!(status.dropped >= 576);
            let mut cursor = 0;
            assert_eq!(drain(&mut cursor, MAX_EVENTS).len(), 1024);
            clear();
        }
    }
}
