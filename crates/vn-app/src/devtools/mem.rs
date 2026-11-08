//! `mem` — where this process's memory is (m26 perf work: "Argon uses a huge
//! amount of RAM").
//!
//! Every number here is computed when asked, from structures that already
//! exist — nothing is sampled per frame. Three kinds of number, and the reply
//! says which is which:
//!
//! * **measured by the OS**: the process counters (`K32GetProcessMemoryInfo`),
//!   a walk of the address space (`VirtualQuery`: committed bytes by mapping
//!   type, executable private pages — JSC's JIT — and the image pages per
//!   module, the mapped files by name), the heaps' own summaries
//!   (`HeapSummary`), and the GPU memory DXGI attributes to this process per
//!   adapter (`QueryVideoMemoryInfo`, one row per adapter LUID);
//! * **counted by an allocator**: the Rust heap (`crate::heapcount` — on by
//!   default in a dev build), JSC's GC heap (`heap_stats`) and JSC's malloc
//!   (mimalloc's own statistics);
//! * **computed from live structures**: gpui's sprite atlas pages and its
//!   decoded-image cache (fork accessors), the retained tree, the image
//!   registry, the devtools' own rings. These are lower bounds.
//!
//! `breakdown` folds all of it into categories that SUM to the committed
//! address space (private + mapped + image), so nothing hides: whatever no
//! allocator accounts for is its own category, "other private", labelled as
//! what it is. It is in BYTES (the inspector's Performance panel formats it);
//! the sections beside it are in MB for a person reading `ctl mem`, and every
//! field named `…Bytes` is bytes.
//!
//! `--gc` first runs redjs's `release_memory` (two full synchronous
//! collections, then free pages back to the OS) and reports the JSC heap and
//! the process counters before and after it. It changes state: finalizers run,
//! and whatever only the GC was holding (fetch `Response` bodies, Rust-side
//! image buffers behind collected JS wrappers) is freed.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;

use gpui::{App, Window};
use vn_runtime::JsRuntime;

use crate::json::Json;

// ---------------------------------------------------------------------------
// the OS: kernel32, declared here rather than through `windows` features the
// graph does not otherwise compile
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCountersEx2 {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
    private_working_set_size: usize,
    shared_commit_usage: u64,
}

#[repr(C)]
struct MemoryBasicInformation {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

#[repr(C)]
#[derive(Default)]
struct HeapSummaryInfo {
    cb: u32,
    allocated: usize,
    committed: usize,
    reserved: usize,
    max_reserve: usize,
}

#[repr(C)]
#[derive(Default)]
struct ThreadBasicInformation {
    exit_status: i32,
    teb: usize,
    client_id: [usize; 2],
    affinity: usize,
    priority: i32,
    base_priority: i32,
}

/// `PROCESS_HEAP_ENTRY` (x64): the region / block union is 24 bytes.
#[repr(C)]
struct ProcessHeapEntry {
    data: *mut c_void,
    size: u32,
    overhead: u8,
    region_index: u8,
    flags: u16,
    region_committed: u32,
    region_uncommitted: u32,
    first_block: *mut c_void,
    last_block: *mut c_void,
}

#[link(name = "kernel32")]
extern "system" {
    fn CloseHandle(handle: isize) -> i32;
    fn ReadProcessMemory(
        process: isize,
        base: *const c_void,
        buf: *mut c_void,
        size: usize,
        read: *mut usize,
    ) -> i32;
    fn HeapLock(heap: isize) -> i32;
    fn HeapUnlock(heap: isize) -> i32;
    fn HeapWalk(heap: isize, entry: *mut ProcessHeapEntry) -> i32;
    fn GetThreadDescription(thread: isize, description: *mut *mut u16) -> i32;
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationThread(
        thread: isize,
        class: u32,
        info: *mut c_void,
        len: u32,
        ret: *mut u32,
    ) -> i32;
    fn NtGetNextThread(
        process: isize,
        thread: isize,
        access: u32,
        attributes: u32,
        flags: u32,
        next: *mut isize,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(
        process: isize,
        counters: *mut ProcessMemoryCountersEx2,
        cb: u32,
    ) -> i32;
    fn K32GetMappedFileNameW(
        process: isize,
        address: *const c_void,
        name: *mut u16,
        size: u32,
    ) -> u32;
    fn VirtualQuery(
        address: *const c_void,
        info: *mut MemoryBasicInformation,
        length: usize,
    ) -> usize;
    fn GetProcessHeap() -> isize;
    fn GetProcessHeaps(count: u32, heaps: *mut isize) -> u32;
    fn HeapSummary(heap: isize, flags: u32, summary: *mut HeapSummaryInfo) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
}

// mimalloc is JSC's allocator here (redjs compiles it into its static lib).
extern "C" {
    fn mi_stats_get_json(buf_size: usize, buf: *mut std::ffi::c_char) -> *mut std::ffi::c_char;
    /// Is `p` inside a mimalloc page? Safe for any pointer (a page-map lookup).
    fn mi_is_in_heap_region(p: *const c_void) -> bool;
}

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_IMAGE: u32 = 0x100_0000;
const MEM_MAPPED: u32 = 0x4_0000;
const MEM_PRIVATE: u32 = 0x2_0000;
const PAGE_EXECUTE_ANY: u32 = 0x10 | 0x20 | 0x40 | 0x80;
const PAGE_GUARD: u32 = 0x100;

fn mb(bytes: f64) -> Json {
    Json::n((bytes / 1048576.0 * 10.0).round() / 10.0)
}

fn bytes(v: f64) -> Json {
    Json::n(v.max(0.0).round())
}

fn ms(t: std::time::Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

// ---------------------------------------------------------------------------
// the OS's view
// ---------------------------------------------------------------------------

struct Counters {
    working_set: f64,
    private_working_set: f64,
    private_bytes: f64,
    json: Json,
}

/// The process counters.
fn process() -> Counters {
    let mut c = ProcessMemoryCountersEx2 { cb: std::mem::size_of::<ProcessMemoryCountersEx2>() as u32, ..Default::default() };
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } != 0;
    if !ok {
        return Counters {
            working_set: 0.0,
            private_working_set: 0.0,
            private_bytes: 0.0,
            json: Json::Null,
        };
    }
    let private = c.private_usage as f64;
    Counters {
        working_set: c.working_set_size as f64,
        private_working_set: c.private_working_set_size as f64,
        private_bytes: private,
        json: Json::obj(vec![
            ("workingSetMB", mb(c.working_set_size as f64)),
            ("privateWorkingSetMB", mb(c.private_working_set_size as f64)),
            (
                "sharedWorkingSetMB",
                mb(c.working_set_size
                    .saturating_sub(c.private_working_set_size) as f64),
            ),
            ("privateBytesMB", mb(private)),
            ("peakWorkingSetMB", mb(c.peak_working_set_size as f64)),
            ("peakPrivateBytesMB", mb(c.peak_pagefile_usage as f64)),
            ("pageFaults", Json::n(f64::from(c.page_fault_count))),
        ]),
    }
}


/// Who a committed private allocation (one `VirtualAlloc` reservation) belongs
/// to, as far as this process can tell from the outside.
#[derive(Clone, Copy, PartialEq)]
enum Owner {
    /// A thread's stack (its TEB's `StackLimit` is inside it).
    Stack,
    /// mimalloc's: some committed page in it is in mimalloc's page map.
    Mimalloc,
    /// The process heap's: a segment or a big block `HeapWalk` reported
    /// (only with `regions`, which walks the heap).
    Heap,
    /// Executable and nobody else's: JSC's JIT pool.
    Jit,
    /// None of the above: driver heaps, other `VirtualAlloc` users — and, when
    /// the heap was not walked, the process heap's segments.
    Unattributed,
}

/// Which breakdown row a resident page is counted under.
#[derive(Clone, Copy, PartialEq)]
enum Resident {
    Image,
    Mapped,
    /// mimalloc's reservations and the JIT's.
    Jsc,
    /// The process heap's (known only when `regions` walked it).
    Heap,
    Stack,
    Unattributed,
}

/// Resident bytes per [`Resident`] class: `[class][0 = private, 1 = shared]`.
type ResidentBytes = [[f64; 2]; 6];

/// Every page of the working set (`QueryWorkingSet`), counted against the
/// region it is in. This is the RAM picture — Task Manager's "Memory" column is
/// the private half of it — where the breakdown's committed bytes are the
/// commit-charge picture: a thread stack's committed-but-untouched pages are in
/// the second and not in the first.
fn resident(w: &Walk) -> Option<ResidentBytes> {
    extern "system" {
        fn K32QueryWorkingSet(process: isize, buf: *mut c_void, size: u32) -> i32;
    }
    let process = unsafe { GetCurrentProcess() };
    let mut buf: Vec<usize> = vec![0; 1 << 18];
    for _ in 0..4 {
        let ok = unsafe { K32QueryWorkingSet(process, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32) } != 0;
        let n = buf[0];
        if ok && n < buf.len() {
            let mut out: ResidentBytes = [[0.0; 2]; 6];
            for &e in &buf[1..=n] {
                let page = e & !0xfff;
                let shared = (e >> 8) & 1;
                let i = w.ranges.partition_point(|r| r.0 <= page);
                let class = if i > 0 && page < w.ranges[i - 1].1 { w.ranges[i - 1].2 } else { Resident::Unattributed };
                out[class as usize][shared] += 4096.0;
            }
            return Some(out);
        }
        // Too small: the first word says how many entries there are now.
        buf = vec![0; n.max(buf.len()) * 5 / 4 + 1024];
    }
    None
}

/// The address-space walk.
struct Walk {
    private: usize,
    mapped: usize,
    image: usize,
    exec: usize,
    reserved: usize,
    regions: usize,
    /// Every image module by committed bytes, biggest first.
    modules: Vec<(String, usize)>,
    /// Every mapped file by committed bytes, biggest first ("" = pagefile-backed).
    mapped_files: Vec<(String, usize)>,
    /// Committed private bytes per [`Owner`].
    by_owner: [usize; 5],
    threads: usize,
    /// `(base, committed, reserved, executable)` of the biggest unattributed
    /// reservations, biggest first.
    unattributed: Vec<(usize, usize, usize, bool)>,
    /// Thread stacks by thread group: `(group, threads, committed, reserved)`, biggest first.
    stack_groups: Vec<(String, usize, usize, usize)>,
    /// Every committed region `(start, end, class)`, in address order.
    ranges: Vec<(usize, usize, Resident)>,
}

impl Walk {
    fn owned(&self, o: Owner) -> usize {
        self.by_owner[o as usize]
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
}

fn allocation_base(p: usize) -> Option<usize> {
    let mut info = std::mem::MaybeUninit::<MemoryBasicInformation>::zeroed();
    let n = unsafe {
        VirtualQuery(
            p as *const c_void,
            info.as_mut_ptr(),
            std::mem::size_of::<MemoryBasicInformation>(),
        )
    };
    (n != 0).then(|| unsafe { info.assume_init() }.allocation_base as usize)
}

/// A thread's description (`SetThreadDescription` — Rust's `thread::Builder`
/// names, JSC's, gpui's), with every run of digits folded to `#` so a pool's
/// workers group into one row.
fn thread_group(h: isize) -> String {
    let mut p: *mut u16 = std::ptr::null_mut();
    if unsafe { GetThreadDescription(h, &mut p) } < 0 || p.is_null() {
        return "(unnamed)".into();
    }
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 && n < 256 {
        n += 1;
    }
    let raw = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) });
    unsafe { LocalFree(p.cast()) };
    let mut out = String::new();
    for c in raw.trim().chars() {
        if c.is_ascii_digit() {
            if !out.ends_with('#') {
                out.push('#');
            }
        } else {
            out.push(c);
        }
    }
    if out.is_empty() { "(unnamed)".into() } else { out }
}

/// The allocation base of every thread's stack in this process (with the
/// thread's group name), and how many threads there are. A thread's TEB starts with its `NT_TIB`, whose third
/// word is `StackLimit`; the TEB is read with `ReadProcessMemory` on ourselves
/// because a thread can exit between the snapshot and the read.
fn stack_bases() -> (HashMap<usize, String>, usize) {
    const THREAD_QUERY_INFORMATION: u32 = 0x40;
    const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x800;
    let mut bases = HashMap::new();
    let process = unsafe { GetCurrentProcess() };
    let mut threads = 0;
    // `NtGetNextThread` walks THIS process's threads; a Toolhelp snapshot would
    // copy every thread on the machine first (hundreds of ms on a busy one).
    let mut h: isize = 0;
    loop {
        let mut next: isize = 0;
        let status = unsafe {
            NtGetNextThread(
                process,
                h,
                THREAD_QUERY_INFORMATION | THREAD_QUERY_LIMITED_INFORMATION,
                0,
                0,
                &mut next,
            )
        };
        if h != 0 {
            unsafe { CloseHandle(h) };
        }
        if status < 0 || next == 0 {
            break;
        }
        h = next;
        threads += 1;
        let name = thread_group(h);
        let mut tbi = ThreadBasicInformation::default();
        let status = unsafe {
            NtQueryInformationThread(
                h,
                0, // ThreadBasicInformation
                (&mut tbi as *mut ThreadBasicInformation).cast(),
                std::mem::size_of::<ThreadBasicInformation>() as u32,
                std::ptr::null_mut(),
            )
        };
        if status >= 0 && tbi.teb != 0 {
            let mut tib = [0usize; 3];
            let mut read = 0usize;
            let ok = unsafe {
                ReadProcessMemory(
                    process,
                    tbi.teb as *const c_void,
                    tib.as_mut_ptr().cast(),
                    24,
                    &mut read,
                )
            } != 0;
            if ok && tib[2] != 0 {
                if let Some(base) = allocation_base(tib[2]) {
                    bases.insert(base, name);
                }
            }
        }
    }
    (bases, threads)
}


/// The allocation bases the process heap's segments and big blocks live in,
/// from a `HeapWalk` under `HeapLock` — every block is visited, so this is the
/// expensive part of `regions` (tens of ms on a heap of 300k blocks) and the
/// heap is locked for that long. Nothing allocates while it is locked: the
/// addresses go into a buffer sized beforehand.
fn heap_bases() -> (HashSet<usize>, usize, f64) {
    const PROCESS_HEAP_REGION: u16 = 0x1;
    const PROCESS_HEAP_ENTRY_BUSY: u16 = 0x4;
    let t = std::time::Instant::now();
    let heap = unsafe { GetProcessHeap() };
    let mut addrs: Vec<usize> = Vec::with_capacity(1 << 16);
    let mut entries = 0usize;
    let mut e = ProcessHeapEntry {
        data: std::ptr::null_mut(),
        size: 0,
        overhead: 0,
        region_index: 0,
        flags: 0,
        region_committed: 0,
        region_uncommitted: 0,
        first_block: std::ptr::null_mut(),
        last_block: std::ptr::null_mut(),
    };
    unsafe { HeapLock(heap) };
    while unsafe { HeapWalk(heap, &mut e) } != 0 {
        entries += 1;
        let big_block = e.flags & PROCESS_HEAP_ENTRY_BUSY != 0 && e.size >= 256 * 1024;
        if (e.flags & PROCESS_HEAP_REGION != 0 || big_block) && addrs.len() < addrs.capacity() {
            addrs.push(e.data as usize);
        }
    }
    unsafe { HeapUnlock(heap) };
    let bases = addrs.into_iter().filter_map(allocation_base).collect();
    (bases, entries, t.elapsed().as_secs_f64() * 1000.0)
}

#[derive(Default)]
struct Base {
    committed: usize,
    reserved: usize,
    exec: usize,
    mimalloc: bool,
}

fn walk(stacks: &HashMap<usize, String>, threads: usize, heap: Option<&HashSet<usize>>) -> Walk {
    let (mut private, mut mapped, mut image, mut exec, mut reserved) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut modules: HashMap<usize, usize> = HashMap::new();
    let mut mapped_bases: HashMap<usize, usize> = HashMap::new();
    let mut private_bases: HashMap<usize, Base> = HashMap::new();
    // Every committed region `(start, size, allocation base, kind)`, in address
    // order — what `resident` maps working-set pages onto.
    let mut committed: Vec<(usize, usize, usize, u32)> = Vec::new();
    let mut regions = 0usize;
    let mut at: usize = 0x10000;
    loop {
        let mut info = std::mem::MaybeUninit::<MemoryBasicInformation>::zeroed();
        let n = unsafe {
            VirtualQuery(
                at as *const c_void,
                info.as_mut_ptr(),
                std::mem::size_of::<MemoryBasicInformation>(),
            )
        };
        if n == 0 {
            break;
        }
        let info = unsafe { info.assume_init() };
        regions += 1;
        let size = info.region_size;
        let base = info.allocation_base as usize;
        if info.state == MEM_COMMIT {
            committed.push((info.base_address as usize, size, base, info.kind));
            match info.kind {
                MEM_PRIVATE => {
                    private += size;
                    let b = private_bases.entry(base).or_default();
                    b.committed += size;
                    b.reserved += size;
                    if info.protect & PAGE_EXECUTE_ANY != 0 && info.protect & PAGE_GUARD == 0 {
                        exec += size;
                        b.exec += size;
                    }
                    if !b.mimalloc && info.protect & PAGE_GUARD == 0 {
                        b.mimalloc = unsafe { mi_is_in_heap_region(info.base_address) };
                    }
                }
                MEM_MAPPED => {
                    mapped += size;
                    *mapped_bases.entry(base).or_default() += size;
                }
                MEM_IMAGE => {
                    image += size;
                    *modules.entry(base).or_default() += size;
                }
                _ => {}
            }
        } else if info.state == MEM_RESERVE {
            reserved += size;
            if info.kind == MEM_PRIVATE {
                private_bases.entry(base).or_default().reserved += size;
            }
        }
        let next = (info.base_address as usize).saturating_add(size);
        if next <= at {
            break;
        }
        at = next;
    }

    let mut by_owner = [0usize; 5];
    let mut unattributed = Vec::new();
    let mut owners: HashMap<usize, Owner> = HashMap::new();
    let mut groups: HashMap<&str, (usize, usize, usize)> = HashMap::new();
    for (base, b) in &private_bases {
        if b.committed == 0 {
            continue;
        }
        let owner = if let Some(name) = stacks.get(base) {
            let g = groups.entry(name.as_str()).or_insert((0, 0, 0));
            g.0 += 1;
            g.1 += b.committed;
            g.2 += b.reserved;
            Owner::Stack
        } else if b.mimalloc {
            Owner::Mimalloc
        } else if heap.is_some_and(|h| h.contains(base)) {
            Owner::Heap
        } else if b.exec > 0 {
            Owner::Jit
        } else {
            Owner::Unattributed
        };
        by_owner[owner as usize] += b.committed;
        owners.insert(*base, owner);
        if owner == Owner::Unattributed {
            unattributed.push((*base, b.committed, b.reserved, b.exec > 0));
        }
    }
    unattributed.sort_by(|a, b| b.1.cmp(&a.1));
    unattributed.truncate(16);
    let mut stack_groups: Vec<(String, usize, usize, usize)> =
        groups.into_iter().map(|(name, (n, c, r))| (name.to_string(), n, c, r)).collect();
    stack_groups.sort_by(|a, b| b.2.cmp(&a.2));
    let ranges = committed
        .into_iter()
        .map(|(start, size, base, kind)| {
            let class = match kind {
                MEM_IMAGE => Resident::Image,
                MEM_MAPPED => Resident::Mapped,
                _ => match owners.get(&base) {
                    Some(Owner::Stack) => Resident::Stack,
                    Some(Owner::Mimalloc | Owner::Jit) => Resident::Jsc,
                    Some(Owner::Heap) => Resident::Heap,
                    _ => Resident::Unattributed,
                },
            };
            (start, start + size, class)
        })
        .collect();

    let mut named: Vec<(String, usize)> = modules
        .into_iter()
        .map(|(base, bytes)| {
            let mut buf = [0u16; 260];
            let n = unsafe { GetModuleFileNameW(base as *mut c_void, buf.as_mut_ptr(), buf.len() as u32) } as usize;
            (file_name(&String::from_utf16_lossy(&buf[..n])), bytes)
        })
        .collect();
    named.sort_by(|a, b| b.1.cmp(&a.1));
    let mut files: HashMap<String, usize> = HashMap::new();
    let process = unsafe { GetCurrentProcess() };
    for (base, bytes) in mapped_bases {
        let mut buf = [0u16; 520];
        let n = unsafe {
            K32GetMappedFileNameW(
                process,
                base as *const c_void,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        } as usize;
        // No name: a pagefile-backed section (shared memory — the D3D runtime's,
        // the compositor's), which has no file behind it.
        let name = if n == 0 { String::new() } else { file_name(&String::from_utf16_lossy(&buf[..n])) };
        *files.entry(name).or_default() += bytes;
    }
    let mut mapped_files: Vec<(String, usize)> = files.into_iter().collect();
    mapped_files.sort_by(|a, b| b.1.cmp(&a.1));
    Walk {
        private,
        mapped,
        image,
        exec,
        reserved,
        regions,
        modules: named,
        mapped_files,
        by_owner,
        threads,
        unattributed,
        stack_groups,
        ranges,
    }
}

fn address_json(w: &Walk, heap_walk: Option<(usize, f64)>) -> Json {
    let top: Vec<Json> = w
        .modules
        .iter()
        .take(8)
        .map(|(name, bytes)| {
            Json::obj(vec![
                ("module", Json::s(name.clone())),
                ("committedMB", mb(*bytes as f64)),
            ])
        })
        .collect();
    let mut o = vec![
        ("privateCommittedMB", mb(w.private as f64)),
        ("privateExecutableMB", mb(w.exec as f64)),
        ("mappedCommittedMB", mb(w.mapped as f64)),
        ("imageCommittedMB", mb(w.image as f64)),
        ("reservedMB", mb(w.reserved as f64)),
        ("regions", Json::n(w.regions as f64)),
        ("modules", Json::n(w.modules.len() as f64)),
        ("threads", Json::n(w.threads as f64)),
        ("imageModules", Json::arr(top)),
        (
            "stacks",
            Json::arr(
                w.stack_groups
                    .iter()
                    .map(|(name, n, committed, reserved)| {
                        Json::obj(vec![
                            ("threads", Json::s(name.clone())),
                            ("count", Json::n(*n as f64)),
                            ("committedBytes", bytes(*committed as f64)),
                            ("reservedBytes", bytes(*reserved as f64)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            // Committed private bytes by who owns the reservation (see `Owner`).
            // `heapMB` is 0 unless `regions` walked the heap; the heap's
            // reservations are then in `unattributedMB`.
            "privateByOwnerMB",
            Json::obj(vec![
                ("stacks", mb(w.owned(Owner::Stack) as f64)),
                ("mimalloc", mb(w.owned(Owner::Mimalloc) as f64)),
                ("heap", mb(w.owned(Owner::Heap) as f64)),
                ("jit", mb(w.owned(Owner::Jit) as f64)),
                ("unattributed", mb(w.owned(Owner::Unattributed) as f64)),
            ]),
        ),
    ];
    if let Some((entries, took)) = heap_walk {
        o.push((
            "heapWalk",
            Json::obj(vec![
                ("entries", Json::n(entries as f64)),
                ("ms", Json::n((took * 10.0).round() / 10.0)),
            ]),
        ));
        o.push((
            "largestUnattributed",
            Json::arr(
                w.unattributed
                    .iter()
                    .map(|(base, committed, reserved, exec)| {
                        Json::obj(vec![
                            ("base", Json::s(format!("{base:#x}"))),
                            ("committedBytes", bytes(*committed as f64)),
                            ("reservedBytes", bytes(*reserved as f64)),
                            ("executable", Json::Bool(*exec)),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    Json::obj(o)
}

struct Heaps {
    main_allocated: f64,
    main_committed: f64,
    other_committed: f64,
    json: Json,
}

/// `HeapSummary` of the process heap (the Rust heap and the static CRT's
/// `malloc` share it) and of every other heap.
fn heaps() -> Heaps {
    let mut handles = [0isize; 256];
    let n = unsafe { GetProcessHeaps(handles.len() as u32, handles.as_mut_ptr()) } as usize;
    let main = unsafe { GetProcessHeap() };
    let (mut other_n, mut other_committed, mut other_allocated) = (0usize, 0usize, 0usize);
    let mut main_json = Json::Null;
    let (mut main_allocated, mut main_committed) = (0.0, 0.0);
    for &h in &handles[..n.min(handles.len())] {
        let mut s = HeapSummaryInfo { cb: std::mem::size_of::<HeapSummaryInfo>() as u32, ..Default::default() };
        if unsafe { HeapSummary(h, 0, &mut s) } == 0 {
            continue;
        }
        if h == main {
            main_allocated = s.allocated as f64;
            main_committed = s.committed as f64;
            main_json = Json::obj(vec![
                ("allocatedMB", mb(s.allocated as f64)),
                ("committedMB", mb(s.committed as f64)),
                ("reservedMB", mb(s.reserved as f64)),
            ]);
        } else {
            other_n += 1;
            other_committed += s.committed;
            other_allocated += s.allocated;
        }
    }
    Heaps {
        main_allocated,
        main_committed,
        other_committed: other_committed as f64,
        json: Json::obj(vec![
            ("processHeap", main_json),
            ("otherHeaps", Json::n(other_n as f64)),
            ("otherHeapsAllocatedMB", mb(other_allocated as f64)),
            ("otherHeapsCommittedMB", mb(other_committed as f64)),
        ]),
    }
}

/// One `"name": { "total": .., "peak": .., "current": N }` out of mimalloc's
/// JSON, by name.
fn mi_current(json: &str, name: &str) -> Option<f64> {
    let at = json.find(&format!("\"{name}\":"))?;
    let rest = &json[at..];
    let cur = rest.find("\"current\":")? + "\"current\":".len();
    let digits: String = rest[cur..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().ok()
}

struct Mimalloc {
    committed: f64,
    /// Live allocations: `malloc_normal` + `malloc_huge`.
    live: f64,
    json: Json,
}

fn mimalloc() -> Mimalloc {
    let mut buf = vec![0u8; 64 * 1024];
    let p = unsafe { mi_stats_get_json(buf.len(), buf.as_mut_ptr().cast()) };
    if p.is_null() {
        return Mimalloc {
            committed: 0.0,
            live: 0.0,
            json: Json::Null,
        };
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let text = String::from_utf8_lossy(&buf[..len]);
    let committed = mi_current(&text, "committed").unwrap_or(0.0);
    let normal = mi_current(&text, "malloc_normal").unwrap_or(0.0);
    let huge = mi_current(&text, "malloc_huge").unwrap_or(0.0);
    Mimalloc {
        committed,
        live: normal + huge,
        json: Json::obj(vec![
            ("committedMB", mb(committed)),
            (
                "reservedMB",
                mb(mi_current(&text, "reserved").unwrap_or(0.0)),
            ),
            ("mallocNormalMB", mb(normal)),
            ("mallocHugeMB", mb(huge)),
            ("pages", Json::n(mi_current(&text, "pages").unwrap_or(0.0))),
            (
                "threads",
                Json::n(mi_current(&text, "threads").unwrap_or(0.0)),
            ),
        ]),
    }
}

struct Jsc {
    size: f64,
    capacity: f64,
    extra: f64,
    json: Json,
}

fn jsc_stats() -> Jsc {
    JsRuntime::with(|rt| {
        let s = rt.ctx().heap_stats();
        // What the pump's between-wave memory work costs (redjs web.rs): the
        // RunLoop turns that fire JSC's GC / sweeper timers, and the JS
        // thread's mimalloc idle sweeps.
        let w = vn_runtime::redjs::web::idle_work_stats();
        let ms = |us: u64| Json::n((us as f64 / 1000.0 * 10.0).round() / 10.0);
        let idle_work = Json::obj(vec![
            ("runLoopMs", ms(w.run_loop_us)),
            ("runLoopMaxMs", ms(w.run_loop_max_us)),
            ("sweeps", Json::n(w.sweeps as f64)),
            ("sweepMs", ms(w.sweep_us)),
            ("sweepMaxMs", ms(w.sweep_max_us)),
        ]);
        Jsc {
            size: s.heap_size as f64,
            capacity: s.heap_capacity as f64,
            extra: s.extra_memory as f64,
            json: Json::obj(vec![
                ("heapSizeMB", mb(s.heap_size as f64)),
                ("heapCapacityMB", mb(s.heap_capacity as f64)),
                ("extraMemoryMB", mb(s.extra_memory as f64)),
                ("objects", Json::n(s.object_count as f64)),
                ("protected", Json::n(s.protected_object_count as f64)),
                ("globals", Json::n(s.global_object_count as f64)),
                ("idleWork", idle_work),
            ]),
        }
    })
}

// ---------------------------------------------------------------------------
// the GPU
// ---------------------------------------------------------------------------

/// `(name, LUID, hardware identity, adapter)`.
type Adapter = (
    String,
    (u32, i32),
    [u32; 4],
    windows::Win32::Graphics::Dxgi::IDXGIAdapter3,
);

thread_local! {
    /// The adapters, enumerated once (a DXGI factory per `mem` costs
    /// milliseconds) and de-duplicated by LUID.
    static ADAPTERS: RefCell<Option<Vec<Adapter>>> = const { RefCell::new(None) };
}

fn adapters() -> Vec<Adapter> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1};
    ADAPTERS.with(|cell| {
        cell.borrow_mut()
            .get_or_insert_with(|| {
                let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
                    return Vec::new();
                };
                let mut out: Vec<Adapter> = Vec::new();
                for i in 0.. {
                    let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
                        break;
                    };
                    let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                        continue;
                    };
                    let luid = (desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart);
                    if out.iter().any(|(_, l, ..)| *l == luid) {
                        continue;
                    }
                    let n = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
                    let name = String::from_utf16_lossy(&desc.Description[..n]);
                    let hw = [desc.VendorId, desc.DeviceId, desc.SubSysId, desc.Revision];
                    let Ok(a3) = adapter.cast::<IDXGIAdapter3>() else {
                        continue;
                    };
                    out.push((name, luid, hw, a3));
                }
                out
            })
            .clone()
    })
}

struct Gpu {
    dedicated: f64,
    shared: f64,
    json: Json,
}

/// Per adapter: what DXGI says this PROCESS has resident in the adapter's
/// local (dedicated VRAM) and non-local (shared system memory) segments.
fn gpu() -> Gpu {
    use windows::Win32::Graphics::Dxgi::{
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
    };
    let mut out = Vec::new();
    let (mut dedicated, mut shared_total) = (0.0, 0.0);
    // The same physical GPU can sit behind several LUIDs (this machine: one
    // RTX card, three LUIDs), and asking each for THIS process's usage
    // returns the same counter three times. So a LUID whose hardware identity
    // AND usage match one already reported is that one again, not more VRAM.
    let mut seen: Vec<([u32; 4], u64, u64)> = Vec::new();
    for (name, luid, hw, a3) in adapters() {
        let mut local = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        let mut shared = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        let ok_l = unsafe { a3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut local) }.is_ok();
        let ok_s = unsafe { a3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, &mut shared) }.is_ok();
        if (!ok_l && !ok_s) || (local.CurrentUsage == 0 && shared.CurrentUsage == 0) {
            continue;
        }
        let key = (hw, local.CurrentUsage, shared.CurrentUsage);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        dedicated += local.CurrentUsage as f64;
        shared_total += shared.CurrentUsage as f64;
        out.push(Json::obj(vec![
            ("adapter", Json::s(name)),
            (
                "luid",
                Json::s(format!("{:08x}:{:08x}", luid.1 as u32, luid.0)),
            ),
            ("dedicatedMB", mb(local.CurrentUsage as f64)),
            ("sharedMB", mb(shared.CurrentUsage as f64)),
            ("dedicatedBudgetMB", mb(local.Budget as f64)),
        ]));
    }
    Gpu {
        dedicated,
        shared: shared_total,
        json: Json::arr(out),
    }
}

struct Renderer {
    targets: f64,
    /// Atlas texture bytes per kind: monochrome, polychrome, subpixel.
    atlas: [f64; 3],
    json: Json,
}

/// The primary window's renderer, from gpui (fork accessors) and its size.
fn renderer(window: &Window) -> Renderer {
    let scale = window.scale_factor();
    let size = window.viewport_size();
    let (w, h) = (
        (f32::from(size.width) * scale).round() as f64,
        (f32::from(size.height) * scale).round() as f64,
    );
    // `gpui_windows::directx_renderer`: a 3-buffer BGRA swap chain, a
    // single-sample and a 4x MSAA BGRA path target — 32 bytes a pixel.
    let targets = 32.0 * w * h;
    let mut per_kind = [0.0; 3];
    let atlas_json = match window.sprite_atlas_stats() {
        None => Json::Null,
        Some(a) => {
            let names = ["monochrome", "polychrome", "subpixel"];
            let mut kinds = Vec::new();
            let mut total = 0.0;
            for (i, k) in a.kinds.iter().enumerate() {
                per_kind[i] = k.bytes as f64;
                total += k.bytes as f64;
                let bpp = if i == 0 { 1.0 } else { 4.0 };
                kinds.push((
                    names[i],
                    Json::obj(vec![
                        ("pages", Json::n(f64::from(k.pages))),
                        ("MB", mb(k.bytes as f64)),
                        ("tiles", Json::n(f64::from(k.tiles))),
                        ("usedMB", mb(k.tile_pixels as f64 * bpp)),
                    ]),
                ));
            }
            kinds.push(("totalMB", mb(total)));
            kinds.push(("keys", Json::n(f64::from(a.keys))));
            kinds.push(("imageTiles", Json::n(f64::from(a.image_keys))));
            kinds.push(("imageTilesMB", mb(a.image_pixels as f64 * 4.0)));
            Json::obj(kinds)
        }
    };
    Renderer {
        targets,
        atlas: per_kind,
        json: Json::obj(vec![
            ("devicePixels", Json::arr(vec![Json::n(w), Json::n(h)])),
            ("renderTargetsMB", mb(targets)),
            ("atlas", atlas_json),
        ]),
    }
}

// ---------------------------------------------------------------------------
// what the host's own structures hold
// ---------------------------------------------------------------------------

struct Images {
    decoded: f64,
    copies: f64,
    json: Json,
}

fn images(cx: &App) -> Images {
    let (entries, image_entries, decoded, decoded_bytes) = cx.image_asset_stats();
    let r = vn_tree::devtools::image_stats();
    let res = vn_tree::devtools::residency_stats();
    let largest: Vec<Json> = r
        .largest
        .iter()
        .map(|(nw, nh, dw, dh, b)| {
            Json::obj(vec![
                (
                    "natural",
                    Json::arr(vec![Json::n(f64::from(*nw)), Json::n(f64::from(*nh))]),
                ),
                (
                    "drawn",
                    Json::arr(vec![Json::n(f64::from(*dw)), Json::n(f64::from(*dh))]),
                ),
                ("MB", mb(*b as f64)),
            ])
        })
        .collect();
    Images {
        decoded: decoded_bytes as f64,
        copies: r.copies_bytes as f64,
        json: Json::obj(vec![
            (
                "gpuiAssetCache",
                Json::obj(vec![
                    ("entries", Json::n(entries as f64)),
                    ("imageEntries", Json::n(image_entries as f64)),
                    ("decoded", Json::n(decoded as f64)),
                    ("decodedMB", mb(decoded_bytes as f64)),
                ]),
            ),
            (
                "painted",
                if r.tracking {
                    Json::obj(vec![
                        ("live", Json::n(r.live as f64)),
                        ("liveMB", mb(r.live_bytes as f64)),
                        ("liveAtDrawnSizeMB", mb(r.live_bytes_at_drawn as f64)),
                        ("oversized4x", Json::n(r.oversized as f64)),
                        ("oversized4xMB", mb(r.oversized_bytes as f64)),
                        ("fullSizeInAtlas", Json::n(r.full_in_atlas as f64)),
                        ("fullSizeInAtlasMB", mb(r.full_in_atlas_bytes as f64)),
                        ("gone", Json::n(r.gone as f64)),
                        ("largest", Json::arr(largest)),
                    ])
                } else {
                    Json::s(
                        "off — the Rust heap is not being counted (VN_MEMSTATS=0, or a prod build)",
                    )
                },
            ),
            (
                "resampleCopies",
                Json::obj(vec![
                    ("copies", Json::n(r.copies as f64)),
                    ("MB", mb(r.copies_bytes as f64)),
                    ("inFlight", Json::n(r.copies_in_flight as f64)),
                ]),
            ),
            (
                // vn-tree img.rs "residency": since boot.
                "residency",
                Json::obj(vec![
                    ("known", Json::n(res.known as f64)),
                    ("reduced", Json::n(res.reduced as f64)),
                    ("reducedFreedMB", mb(res.reduced_freed as f64)),
                    ("restored", Json::n(res.restored as f64)),
                    ("evicted", Json::n(res.evicted as f64)),
                    ("evictedFreedMB", mb(res.evicted_freed as f64)),
                    (
                        // Shown decodes the latest sweep left full, by reason.
                        "keptFull",
                        Json::obj(vec![
                            ("notStill", Json::n(res.kept_full.not_still as f64)),
                            ("undrawn", Json::n(res.kept_full.undrawn as f64)),
                            ("young", Json::n(res.kept_full.young as f64)),
                            ("drawnLarge", Json::n(res.kept_full.drawn_large as f64)),
                            ("noCopy", Json::n(res.kept_full.no_copy as f64)),
                            ("smallSaving", Json::n(res.kept_full.small_saving as f64)),
                            ("canvasRead", Json::n(res.kept_full.canvas_read as f64)),
                            (
                                "fullResWindow",
                                Json::n(res.kept_full.full_res_window as f64),
                            ),
                        ]),
                    ),
                ]),
            ),
        ]),
    }
}

fn tree() -> (Json, f64) {
    let m = vn_tree::devtools::tree_mem();
    let per = |b: usize| {
        if m.nodes > 0 {
            Json::n((b as f64 / m.nodes as f64).round())
        } else {
            Json::Null
        }
    };
    (
        Json::obj(vec![
            ("nodes", Json::n(m.nodes as f64)),
            ("elements", Json::n(m.elements as f64)),
            ("texts", Json::n(m.texts as f64)),
            ("anchors", Json::n(m.anchors as f64)),
            ("slots", Json::n(m.slots as f64)),
            ("computed", Json::n(m.computed as f64)),
            ("estimateMB", mb(m.total_bytes as f64)),
            ("bytesPerNode", per(m.total_bytes)),
            (
                "parts",
                Json::obj(vec![
                    ("slabMB", mb(m.slot_bytes as f64)),
                    ("ownedMB", mb(m.owned_bytes as f64)),
                    ("computedStructsMB", mb(m.computed_bytes as f64)),
                    ("refinements", Json::n(m.refinements as f64)),
                    ("refinementsMB", mb(m.refinement_bytes as f64)),
                    ("varsMaps", Json::n(m.vars_maps as f64)),
                    ("varEntries", Json::n(m.var_entries as f64)),
                    ("varsMB", mb(m.vars_bytes as f64)),
                    ("inlineStyles", Json::n(m.inlines as f64)),
                    ("inlineMB", mb(m.inline_bytes as f64)),
                    ("names", Json::n(m.names as f64)),
                ]),
            ),
        ]),
        m.total_bytes as f64,
    )
}

/// vn-tree's side tables and the devtools' own rings: every value is a count
/// or, for a name ending in `Bytes`, BYTES.
fn side() -> Vec<(&'static str, u64)> {
    let mut o = vn_tree::devtools::side_stats();
    let (rows, row_bytes, records, held, meta, evicted) = super::net::mem_stats();
    o.push(("netRows", rows as u64));
    o.push(("netRowBytes", row_bytes as u64));
    o.push(("netRecords", records as u64));
    o.push(("netBodyBytes", held as u64));
    o.push(("netMetaBytes", meta as u64));
    o.push(("netBodiesEvicted", evicted));
    o
}

/// `<vn-sticker>`'s engine: its sources, loaded engines, frames held and loop
/// cache. `null` until something has used it.
fn stickers() -> Json {
    let Some(s) = vn_sticker::stats() else {
        return Json::Null;
    };
    let n = |v: usize| Json::n(v as f64);
    Json::obj(vec![
        ("sources", n(s.sources)),
        ("fileBytes", bytes(s.file_bytes as f64)),
        ("engines", n(s.engines)),
        ("engineBytes", bytes(s.engine_bytes as f64)),
        ("targets", n(s.targets)),
        ("visibleTargets", n(s.visible_targets)),
        ("frameBytes", bytes(s.frame_bytes as f64)),
        ("loops", n(s.loops)),
        ("loopsComplete", n(s.loops_complete)),
        ("cacheBytes", bytes(s.cache_bytes as f64)),
        ("cacheBudgetBytes", bytes(s.cache_budget as f64)),
        ("drawn", Json::n(s.drawn as f64)),
        ("fromCache", Json::n(s.from_cache as f64)),
        ("loads", Json::n(s.loads as f64)),
        ("failures", Json::n(s.failures as f64)),
    ])
}

fn rust_json(rust: Option<(i64, i64, i64, u64)>) -> Json {
    match rust {
        Some((live, blocks, peak, allocs)) => {
            let classes: Vec<Json> = crate::heapcount::classes()
                .into_iter()
                .map(|(upper, b, n)| {
                    Json::obj(vec![
                        (
                            "upTo",
                            if upper == u64::MAX {
                                Json::Null
                            } else {
                                Json::n(upper as f64)
                            },
                        ),
                        ("bytes", bytes(b as f64)),
                        ("blocks", Json::n(n as f64)),
                    ])
                })
                .collect();
            Json::obj(vec![
                ("liveMB", mb(live as f64)),
                ("blocks", Json::n(blocks as f64)),
                ("peakMB", mb(peak as f64)),
                ("allocations", Json::n(allocs as f64)),
                ("sizeClasses", Json::arr(classes)),
            ])
        }
        None => Json::s("off — VN_MEMSTATS=0, or a prod build (VN_MEMSTATS=1 turns it on)"),
    }
}

// ---------------------------------------------------------------------------
// the breakdown
// ---------------------------------------------------------------------------

fn item(label: impl Into<String>, b: f64) -> Json {
    Json::obj(vec![("label", Json::s(label)), ("bytes", bytes(b))])
}

/// A row that is NOT a slice of its category — a figure from another
/// accountant (JSC's own GC heap count) shown beside the slices it overlaps.
fn info(label: impl Into<String>, b: f64) -> Json {
    Json::obj(vec![
        ("label", Json::s(label)),
        ("bytes", bytes(b)),
        ("info", Json::Bool(true)),
    ])
}

fn category(key: &str, label: &str, b: f64, note: &str, items: Vec<Json>) -> Json {
    Json::obj(vec![
        ("key", Json::s(key)),
        ("label", Json::s(label)),
        ("bytes", bytes(b)),
        ("note", Json::s(note)),
        ("items", Json::arr(items)),
    ])
}

/// Adds a category's resident bytes (all, and private) when they were counted.
fn with_resident(mut c: Json, r: Option<[f64; 2]>) -> Json {
    if let (Json::Obj(fields), Some([private, shared])) = (&mut c, r) {
        fields.push(("residentBytes".into(), bytes(private + shared)));
        fields.push(("residentPrivateBytes".into(), bytes(private)));
    }
    c
}

struct Inputs<'a> {
    resident: Option<&'a ResidentBytes>,
    counters: &'a Counters,
    walk: &'a Walk,
    heaps: &'a Heaps,
    mi: &'a Mimalloc,
    jsc: &'a Jsc,
    gpu: &'a Gpu,
    renderer: &'a Renderer,
    images: &'a Images,
    tree_bytes: f64,
    side: &'a [(&'static str, u64)],
    rust_live: Option<f64>,
}

/// Categories that sum to the committed address space, in bytes.
fn breakdown(i: &Inputs) -> Json {
    let w = i.walk;
    let side = |name: &str| {
        i.side
            .iter()
            .find(|(k, _)| *k == name)
            .map_or(0.0, |(_, v)| *v as f64)
    };

    // Images: the exe and every DLL, the biggest named.
    let mut image_items: Vec<Json> = w.modules.iter().take(10).map(|(n, b)| item(n.clone(), *b as f64)).collect();
    if w.modules.len() > 10 {
        let rest: usize = w.modules.iter().skip(10).map(|(_, b)| *b).sum();
        image_items.push(item(
            format!("{} other modules", w.modules.len() - 10),
            rest as f64,
        ));
    }

    // Mapped files.
    let mut mapped_items: Vec<Json> = w
        .mapped_files
        .iter()
        .take(8)
        .map(|(n, b)| {
            item(
                if n.is_empty() {
                    "pagefile-backed sections (shared memory)".to_string()
                } else {
                    n.clone()
                },
                *b as f64,
            )
        })
        .collect();
    if w.mapped_files.len() > 8 {
        let rest: usize = w.mapped_files.iter().skip(8).map(|(_, b)| *b).sum();
        mapped_items.push(item(
            format!("{} other files", w.mapped_files.len() - 8),
            rest as f64,
        ));
    }

    // JSC: its malloc (mimalloc, which the GC heap's blocks come out of too)
    // and its JIT's executable pages.
    let jsc_bytes = i.mi.committed + w.exec as f64;
    let jsc_items = vec![
        item(
            "mimalloc: live allocations (GC heap blocks, strings, bytecode, the bundle's source)",
            i.mi.live.min(i.mi.committed),
        ),
        item(
            "mimalloc: committed, not allocated (pages it has not returned)",
            i.mi.committed - i.mi.live,
        ),
        item("JIT code (executable private pages)", w.exec as f64),
        info("by JSC's own count: GC heap in use", i.jsc.size),
        info("by JSC's own count: GC heap capacity", i.jsc.capacity),
        info(
            "by JSC's own count: extra memory (ArrayBuffers, external strings)",
            i.jsc.extra,
        ),
    ];

    // The process heap: Rust's `System` allocator and the static CRT's malloc.
    let heap = i.heaps.main_committed + i.heaps.other_committed;
    let mut rust_items = Vec::new();
    match i.rust_live {
        Some(live) => {
            let parts = [
                (
                    "retained tree (nodes, styles, vars) — estimate",
                    i.tree_bytes,
                ),
                (
                    "decoded images (gpui asset cache; a reduced one is its copy's size)",
                    i.images.decoded,
                ),
                ("downscaled image copies (resample LRU)", i.images.copies),
                ("canvas surfaces (host copies)", side("canvasHostCopyBytes")),
                ("SVG colour rasters", side("svgColourRasterBytes")),
                (
                    "encoded images held in memory, image src strings",
                    side("imgEncodedInMemoryBytes") + side("imgSrcBytes"),
                ),
                (
                    "web fonts (font file bytes)",
                    side("fontFacesRegisteredBytes") + side("fontFacesLoadedNotRegisteredBytes"),
                ),
                (
                    "devtools network tee (rows, bodies, metadata)",
                    side("netRowBytes") + side("netBodyBytes") + side("netMetaBytes"),
                ),
                ("devtools log ring", side("logRingBytes")),
            ];
            let mut named: f64 = parts.iter().map(|(_, b)| *b).sum();
            for (label, b) in parts {
                rust_items.push(item(label, b));
            }
            // Every stylesheet compile leaks its source and its lightningcss AST
            // (vn-css `Engine::compile`: the rules borrow `'static` selectors
            // out of it), so what compiles leave behind is measured around each
            // one — net of the engine it replaced.
            let (compiles, _, _, growth) = vn_tree::perf::sheet_compiles();
            if let Some(g) = growth {
                rust_items.push(item(
                    format!("stylesheet compiles since boot ({compiles}): what they left behind, net"),
                    g as f64,
                ));
                named += g as f64;
            }
            rust_items.push(item(
                "rest of the Rust heap (gpui, the cascade, text layout caches, the JS bridge — not itemised)",
                live - named,
            ));
            rust_items.push(item(
                "C/C++ malloc on the same heap (allocated − Rust live)",
                i.heaps.main_allocated - live,
            ));
        }
        None => {
            rust_items.push(item(
                "allocated: Rust + C/C++ malloc (Rust heap counting is off)",
                i.heaps.main_allocated,
            ));
        }
    }
    rust_items.push(item(
        "heap slack (committed, not allocated)",
        i.heaps.main_committed - i.heaps.main_allocated,
    ));
    if i.heaps.other_committed > 0.0 {
        rust_items.push(item("other Win32 heaps", i.heaps.other_committed));
    }

    // Whatever no allocator we can see accounts for — less the thread stacks,
    // which the walk finds exactly (each thread's TEB names its stack).
    let other = w.private as f64 - jsc_bytes - heap;
    let stacks = w.owned(Owner::Stack) as f64;
    let mut other_items: Vec<Json> = w
        .stack_groups
        .iter()
        .take(6)
        .map(|(name, n, committed, _)| {
            item(format!("thread stacks: {name} ×{n}"), *committed as f64)
        })
        .collect();
    if w.stack_groups.len() > 6 {
        let (n, c) = w
            .stack_groups
            .iter()
            .skip(6)
            .fold((0, 0), |(n, c), g| (n + g.1, c + g.2));
        other_items.push(item(
            format!("thread stacks: {} other threads", n),
            c as f64,
        ));
    }
    other_items.push(item(
        "rest: D3D / GPU driver user-mode heaps, other VirtualAlloc users",
        other - stacks,
    ));

    // Resident bytes per row (only with `regions`: without the heap walk the
    // heap's pages and the unattributed ones cannot be told apart).
    let res = |classes: &[Resident]| {
        i.resident.map(|r| {
            classes.iter().fold([0.0, 0.0], |acc, c| {
                [acc[0] + r[*c as usize][0], acc[1] + r[*c as usize][1]]
            })
        })
    };
    if let Some(r) = i.resident {
        other_items.push(info(
            "resident: thread stacks",
            r[Resident::Stack as usize][0] + r[Resident::Stack as usize][1],
        ));
    }

    let total = (w.private + w.mapped + w.image) as f64;
    let cats = vec![
        with_resident(
            category(
                "images",
                "Loaded images (exe + DLLs)",
                w.image as f64,
                "file-backed and mostly shared: counted in the working set, not in private bytes",
                image_items,
            ),
            res(&[Resident::Image]),
        ),
        with_resident(
            category(
                "mapped",
                "Mapped files",
                w.mapped as f64,
                "fonts, locale data, shared sections",
                mapped_items,
            ),
            res(&[Resident::Mapped]),
        ),
        with_resident(
            category(
                "jsc",
                "JSC",
                jsc_bytes,
                "mimalloc (JSC's malloc) + JIT code",
                jsc_items,
            ),
            res(&[Resident::Jsc]),
        ),
        with_resident(
            category(
                "rust",
                "Rust heap (process heap)",
                heap,
                "the process heap: Rust's allocator and the static CRT's malloc share it",
                rust_items,
            ),
            res(&[Resident::Heap]),
        ),
        with_resident(
            category(
                "other",
                "Other private (unattributed)",
                other,
                "committed private memory no allocator we can see accounts for",
                other_items,
            ),
            res(&[Resident::Stack, Resident::Unattributed]),
        ),
    ];
    let a = i.renderer.atlas;
    let gpu_total = i.gpu.dedicated + i.gpu.shared;
    let gpu_named = a.iter().sum::<f64>() + i.renderer.targets;
    Json::obj(vec![
        (
            "basis",
            Json::s("committed address space: private + mapped + image"),
        ),
        ("totalBytes", bytes(total)),
        ("privateBytes", bytes(i.counters.private_bytes)),
        ("workingSetBytes", bytes(i.counters.working_set)),
        (
            "privateWorkingSetBytes",
            bytes(i.counters.private_working_set),
        ),
        ("rustCounting", Json::Bool(i.rust_live.is_some())),
        ("categories", Json::arr(cats)),
        (
            "gpu",
            Json::obj(vec![
                ("dedicatedBytes", bytes(i.gpu.dedicated)),
                ("sharedBytes", bytes(i.gpu.shared)),
                (
                    "items",
                    Json::arr(vec![
                        item("sprite atlas: monochrome (glyphs)", a[0]),
                        item("sprite atlas: polychrome (images, emoji, SVG)", a[1]),
                        item("sprite atlas: subpixel (glyphs)", a[2]),
                        item(
                            "render targets (swap chain + path MSAA, estimate)",
                            i.renderer.targets,
                        ),
                        item(
                            "rest (video surfaces, driver, D3D internals)",
                            gpu_total - gpu_named,
                        ),
                    ]),
                ),
            ]),
        ),
    ])
}

/// The whole report. `gc`: run `release_memory` first and report before/after;
/// `code`: the same, after deleting all compiled code that is not running
/// (`redjs_release_memory_and_code`), which tells code from data.
/// `regions`: also walk the process heap (locked meanwhile), so the address
/// walk can tell its reservations from the unattributed ones, and list the
/// biggest of those.
pub(crate) fn report(
    gc: bool,
    code: bool,
    regions: bool,
    window: &mut Window,
    cx: &mut App,
) -> Json {
    let gc = gc || code;
    let before = if gc { Some((process(), jsc_stats())) } else { None };
    let gc_ms = if gc {
        let t = std::time::Instant::now();
        JsRuntime::with(|rt| unsafe {
            if code {
                vn_runtime::redjs::ffi::redjs_release_memory_and_code(rt.raw())
            } else {
                vn_runtime::redjs::ffi::redjs_release_memory(rt.raw())
            }
        });
        Some(ms(t))
    } else {
        None
    };
    let t = std::time::Instant::now();
    let counters = process();
    let (stacks, threads) = stack_bases();
    let heap_walk = regions.then(heap_bases);
    let walk = walk(&stacks, threads, heap_walk.as_ref().map(|(b, ..)| b));
    let resident = if regions { resident(&walk) } else { None };
    let heaps = heaps();
    let mi = mimalloc();
    let jsc = jsc_stats();
    let gpu = gpu();
    let renderer = renderer(window);
    let images = images(cx);
    let (tree_json, tree_bytes) = tree();
    let side = side();
    let rust = crate::heapcount::snapshot();
    let rust_live = rust.map(|(live, ..)| live as f64);
    let breakdown = breakdown(&Inputs {
        resident: resident.as_ref(),
        counters: &counters,
        walk: &walk,
        heaps: &heaps,
        mi: &mi,
        jsc: &jsc,
        gpu: &gpu,
        renderer: &renderer,
        images: &images,
        tree_bytes,
        side: &side,
        rust_live,
    });
    // What the three allocators we can see account for, against private bytes.
    let explained = heaps.main_committed + mi.committed + walk.exec as f64;
    let side_json = Json::obj(side.iter().map(|(k, v)| (*k, Json::n(*v as f64))).collect());
    let (compiles, compile_ms, compile_source, compile_growth) = vn_tree::perf::sheet_compiles();
    let sheets_json = Json::obj(vec![
        ("compiles", Json::n(compiles as f64)),
        ("ms", Json::n(compile_ms.round())),
        ("sourceBytes", bytes(compile_source as f64)),
        (
            "netGrowthBytes",
            compile_growth.map_or(Json::Null, |g| Json::n(g as f64)),
        ),
    ]);
    let mut o = vec![
        ("breakdown", breakdown),
        ("process", counters.json),
        (
            "addressSpace",
            address_json(&walk, heap_walk.as_ref().map(|(_, n, took)| (*n, *took))),
        ),
        ("heaps", heaps.json),
        ("rustHeap", rust_json(rust)),
        ("mimalloc", mi.json),
        ("jsc", jsc.json),
        ("gpu", gpu.json),
        ("renderer", renderer.json),
        ("images", images.json),
        ("tree", tree_json),
        ("side", side_json),
        ("stickers", stickers()),
        // The disk HTTP cache (`net_cache` has the same block): on disk, not
        // in memory, but it is what makes an evicted image cheap to show again.
        ("httpCache", super::net::cache_json()),
        ("stylesheetCompiles", sheets_json),
        (
            "explained",
            Json::obj(vec![
                ("privateBytesMB", mb(counters.private_bytes)),
                ("processHeapMB", mb(heaps.main_committed)),
                ("mimallocMB", mb(mi.committed)),
                ("jitExecutableMB", mb(walk.exec as f64)),
                ("sumMB", mb(explained)),
                ("unexplainedMB", mb(counters.private_bytes - explained)),
                ("gpuProcessMB", mb(gpu.dedicated + gpu.shared)),
            ]),
        ),
        ("collectMs", Json::n(ms(t))),
    ];
    if let (Some(took), Some((before_counters, before_jsc))) = (gc_ms, before) {
        o.push((
            "gc",
            Json::obj(vec![
                ("ms", Json::n(took)),
                (
                    "before",
                    Json::obj(vec![
                        ("process", before_counters.json),
                        ("jsc", before_jsc.json),
                    ]),
                ),
                (
                    "freedPrivateBytes",
                    bytes(before_counters.private_bytes - counters.private_bytes),
                ),
                ("freedJscHeapBytes", bytes(before_jsc.size - jsc.size)),
            ]),
        ));
    }
    #[cfg(feature = "webgpu")]
    {
        #[cfg(target_os = "windows")]
        if let Some(shared) = crate::shared_gpu::stats() {
            o.push((
                "webgpuNative",
                Json::obj(vec![
                    ("rendererGeneration", Json::n(shared.generation as f64)),
                    ("liveContexts", Json::n(shared.contexts as f64)),
                    ("liveConverters", Json::n(shared.converters as f64)),
                    ("liveFrames", Json::n(shared.frames as f64)),
                    ("videoBridgeActive", Json::Bool(shared.video_bridge_active)),
                ]),
            ));
        }
        let (native, readback, snapshots) =
            vn_runtime::redjs::webapi::webgpu::canvas::presentation_stats();
        let (copies, fallbacks) = vn_tree::devtools::gpu_canvas_presentation_stats();
        o.push((
            "webgpuPresentation",
            Json::obj(vec![
                ("atlasGpuCopies", Json::n(copies as f64)),
                ("atlasCpuFallbacks", Json::n(fallbacks as f64)),
                ("nativeDisplayFrames", Json::n(native as f64)),
                ("readbackDisplayFrames", Json::n(readback as f64)),
                ("explicitSnapshots", Json::n(snapshots as f64)),
            ]),
        ));
    }
    Json::obj(o)
}
