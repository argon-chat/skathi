//! `mem`: where this process's memory is, computed on request.
//!
//! Numbers are measured by the OS, counted by an allocator, or computed from live structures
//! (lower bounds). `breakdown` (bytes) folds them into categories that sum to the committed
//! address space; the other sections are in MB. `gc` first runs `release_memory` and reports
//! before/after, which changes state (finalizers run).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;

use gpui::{App, Window};

use super::mem_common::{
    bytes, category, collect, file_name, gpu_items, images, info, item, jsc_stats, mb, mimalloc,
    ms, renderer, rust_items as rust_items_of, rust_json, side, side_json, stickers,
    stylesheet_compiles, tree, with_resident, Images, Jsc, Mimalloc, Renderer,
};
use crate::json::Json;

// kernel32 declared by hand rather than through `windows` features the graph
// does not otherwise compile.

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
unsafe extern "system" {
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
unsafe extern "system" {
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
unsafe extern "system" {
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

unsafe extern "C" {
    /// Safe for any pointer (a page-map lookup).
    fn mi_is_in_heap_region(p: *const c_void) -> bool;
}

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_IMAGE: u32 = 0x100_0000;
const MEM_MAPPED: u32 = 0x4_0000;
const MEM_PRIVATE: u32 = 0x2_0000;
const PAGE_EXECUTE_ANY: u32 = 0x10 | 0x20 | 0x40 | 0x80;
const PAGE_GUARD: u32 = 0x100;

struct Counters {
    working_set: f64,
    private_working_set: f64,
    private_bytes: f64,
    json: Json,
}

fn process() -> Counters {
    let mut c = ProcessMemoryCountersEx2 {
        cb: std::mem::size_of::<ProcessMemoryCountersEx2>() as u32,
        ..Default::default()
    };
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

/// Who a committed private allocation (one `VirtualAlloc` reservation) belongs to.
#[derive(Clone, Copy, PartialEq)]
enum Owner {
    /// The TEB's `StackLimit` is inside it.
    Stack,
    /// Some committed page in it is in mimalloc's page map.
    Mimalloc,
    /// A segment or big block `HeapWalk` reported (only with `regions`).
    Heap,
    /// Executable and nobody else's: JSC's JIT pool.
    Jit,
    /// Driver heaps, other `VirtualAlloc` users, and the process heap's
    /// segments when the heap was not walked.
    Unattributed,
}

/// Which breakdown row a resident page is counted under.
#[derive(Clone, Copy, PartialEq)]
enum Resident {
    Image,
    Mapped,
    /// mimalloc's reservations and the JIT's.
    Jsc,
    /// Known only when `regions` walked the heap.
    Heap,
    Stack,
    Unattributed,
}

/// Resident bytes per [`Resident`] class: `[class][0 = private, 1 = shared]`.
type ResidentBytes = [[f64; 2]; 6];

/// The RAM picture (`QueryWorkingSet`); the breakdown is the commit picture (a stack's
/// committed-but-untouched pages are only in the latter).
fn resident(w: &Walk) -> Option<ResidentBytes> {
    unsafe extern "system" {
        fn K32QueryWorkingSet(process: isize, buf: *mut c_void, size: u32) -> i32;
    }
    let process = unsafe { GetCurrentProcess() };
    let mut buf: Vec<usize> = vec![0; 1 << 18];
    for _ in 0..4 {
        let ok =
            unsafe { K32QueryWorkingSet(process, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32) }
                != 0;
        let n = buf[0];
        if ok && n < buf.len() {
            let mut out: ResidentBytes = [[0.0; 2]; 6];
            for &e in &buf[1..=n] {
                let page = e & !0xfff;
                let shared = (e >> 8) & 1;
                let i = w.ranges.partition_point(|r| r.0 <= page);
                let class = if i > 0 && page < w.ranges[i - 1].1 {
                    w.ranges[i - 1].2
                } else {
                    Resident::Unattributed
                };
                out[class as usize][shared] += 4096.0;
            }
            return Some(out);
        }
        // Too small: the first word is the current entry count.
        buf = vec![0; n.max(buf.len()) * 5 / 4 + 1024];
    }
    None
}

struct Walk {
    private: usize,
    mapped: usize,
    image: usize,
    exec: usize,
    reserved: usize,
    regions: usize,
    /// Biggest first.
    modules: Vec<(String, usize)>,
    /// Biggest first; "" = pagefile-backed.
    mapped_files: Vec<(String, usize)>,
    /// Committed private bytes per [`Owner`].
    by_owner: [usize; 5],
    threads: usize,
    /// `(base, committed, reserved, executable)`, biggest first.
    unattributed: Vec<(usize, usize, usize, bool)>,
    /// `(group, threads, committed, reserved)`, biggest first.
    stack_groups: Vec<(String, usize, usize, usize)>,
    /// Committed regions `(start, end, class)`, in address order.
    ranges: Vec<(usize, usize, Resident)>,
}

impl Walk {
    fn owned(&self, o: Owner) -> usize {
        self.by_owner[o as usize]
    }
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

/// Digit runs folded to `#`, so a pool's workers group into one row.
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
    if out.is_empty() {
        "(unnamed)".into()
    } else {
        out
    }
}

/// `StackLimit` (third word of the TEB's `NT_TIB`) is read with `ReadProcessMemory` because a
/// thread can exit mid-walk.
fn stack_bases() -> (HashMap<usize, String>, usize) {
    const THREAD_QUERY_INFORMATION: u32 = 0x40;
    const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x800;
    let mut bases = HashMap::new();
    let process = unsafe { GetCurrentProcess() };
    let mut threads = 0;
    // Not a Toolhelp snapshot: that copies every thread on the machine first.
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

/// `HeapWalk` under `HeapLock`: slow, and the heap stays locked, so nothing may allocate
/// meanwhile (addresses go into a preallocated buffer).
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
    let (mut private, mut mapped, mut image, mut exec, mut reserved) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut modules: HashMap<usize, usize> = HashMap::new();
    let mut mapped_bases: HashMap<usize, usize> = HashMap::new();
    let mut private_bases: HashMap<usize, Base> = HashMap::new();
    // `(start, size, allocation base, kind)`, in address order.
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
    let mut stack_groups: Vec<(String, usize, usize, usize)> = groups
        .into_iter()
        .map(|(name, (n, c, r))| (name.to_string(), n, c, r))
        .collect();
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
            let n = unsafe {
                GetModuleFileNameW(base as *mut c_void, buf.as_mut_ptr(), buf.len() as u32)
            } as usize;
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
        // No name: a pagefile-backed section (shared memory).
        let name = if n == 0 {
            String::new()
        } else {
            file_name(&String::from_utf16_lossy(&buf[..n]))
        };
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
            // `heap` is 0 unless `regions` walked the heap.
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

/// The Rust heap and the static CRT's `malloc` share the process heap.
fn heaps() -> Heaps {
    let mut handles = [0isize; 256];
    let n = unsafe { GetProcessHeaps(handles.len() as u32, handles.as_mut_ptr()) } as usize;
    let main = unsafe { GetProcessHeap() };
    let (mut other_n, mut other_committed, mut other_allocated) = (0usize, 0usize, 0usize);
    let mut main_json = Json::Null;
    let (mut main_allocated, mut main_committed) = (0.0, 0.0);
    for &h in &handles[..n.min(handles.len())] {
        let mut s = HeapSummaryInfo {
            cb: std::mem::size_of::<HeapSummaryInfo>() as u32,
            ..Default::default()
        };
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

/// `(name, LUID, hardware identity, adapter)`.
type Adapter = (
    String,
    (u32, i32),
    [u32; 4],
    windows::Win32::Graphics::Dxgi::IDXGIAdapter3,
);

thread_local! {
    /// Enumerated once (a DXGI factory costs milliseconds), de-duplicated by LUID.
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
                    let n = desc
                        .Description
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(desc.Description.len());
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

/// Per adapter: usage in the local (VRAM) and non-local (shared) segments.
fn gpu() -> Gpu {
    use windows::Win32::Graphics::Dxgi::{
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
        DXGI_QUERY_VIDEO_MEMORY_INFO,
    };
    let mut out = Vec::new();
    let (mut dedicated, mut shared_total) = (0.0, 0.0);
    // One physical GPU can sit behind several LUIDs, each returning the same
    // per-process counter: skip a LUID whose hardware identity and usage match.
    let mut seen: Vec<([u32; 4], u64, u64)> = Vec::new();
    for (name, luid, hw, a3) in adapters() {
        let mut local = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        let mut shared = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        let ok_l =
            unsafe { a3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut local) }
                .is_ok();
        let ok_s =
            unsafe { a3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, &mut shared) }
                .is_ok();
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

    let mut image_items: Vec<Json> = w
        .modules
        .iter()
        .take(10)
        .map(|(n, b)| item(n.clone(), *b as f64))
        .collect();
    if w.modules.len() > 10 {
        let rest: usize = w.modules.iter().skip(10).map(|(_, b)| *b).sum();
        image_items.push(item(
            format!("{} other modules", w.modules.len() - 10),
            rest as f64,
        ));
    }

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

    let heap = i.heaps.main_committed + i.heaps.other_committed;
    let mut rust_items = Vec::new();
    match i.rust_live {
        Some(live) => {
            rust_items = rust_items_of(live, i.tree_bytes, i.images, i.side);
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
    let mut gpu_rows = gpu_items(i.renderer);
    gpu_rows.push(item(
        "rest (video surfaces, driver, D3D internals)",
        gpu_total - gpu_named,
    ));
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
                ("items", Json::arr(gpu_rows)),
            ]),
        ),
    ])
}

/// `gc`: `release_memory` first, before/after; `code`: also drop compiled code; `regions`: also
/// walk the process heap (locked meanwhile).
pub(crate) fn report(
    gc: bool,
    code: bool,
    regions: bool,
    window: &mut Window,
    cx: &mut App,
) -> Json {
    let gc = gc || code;
    let before = if gc {
        Some((process(), jsc_stats()))
    } else {
        None
    };
    let gc_ms = gc.then(|| collect(code));
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
    let explained = heaps.main_committed + mi.committed + walk.exec as f64;
    let side_json = side_json(&side);
    let sheets_json = stylesheet_compiles();
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
        let (native, readback, snapshots) = sk_runtime::redjs_webgpu::canvas::presentation_stats();
        let (copies, fallbacks) = sk_tree::devtools::gpu_canvas_presentation_stats();
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
