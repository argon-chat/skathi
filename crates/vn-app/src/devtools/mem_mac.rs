//! `mem` on macOS: the reply `mem.rs` gives on Windows (read its doc for the
//! sections and the three kinds of number), read through Mach instead of
//! kernel32. The breakdown's basis is the task's **physical footprint** —
//! Activity Monitor's "Memory" — rather than a commit charge macOS does not
//! have; `mem_fold.rs` says how regions become categories that sum to it.
//!
//! * **measured by the OS**: `task_info(TASK_VM_INFO)` (footprint, resident,
//!   internal / external, compressed, the graphics ledger), a walk of every VM
//!   region with `mach_vm_region_recurse` into the shared-region submap (each
//!   region's tag, share mode and resident / dirty / compressed / reusable
//!   page counts; `proc_regionfilename` names a file-backed one), the threads
//!   (`task_threads`; a pthread's name and stack range), and the malloc zones'
//!   own summary (`malloc_zone_statistics`);
//! * **counted by an allocator** and **computed from live structures**: the
//!   same as on Windows (`mem_common.rs`).
//!
//! `regions` adds what the walk already has: the resident bytes per category
//! and the largest untagged regions. It walks nothing extra — every page count
//! comes with the region — so on macOS it costs no more than the plain reply.

use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_int, c_long, c_void, CStr};
use std::mem::{offset_of, size_of};

use gpui::{App, Window};
use mach2::dyld::{_dyld_get_image_name, _dyld_image_count};
use mach2::kern_return::KERN_SUCCESS;
use mach2::mach_port::mach_port_deallocate;
use mach2::mach_types::{thread_act_array_t, thread_act_t};
use mach2::message::mach_msg_type_number_t;
use mach2::task::{task_info, task_threads};
use mach2::task_info::{task_vm_info, TASK_VM_INFO};
use mach2::traps::mach_task_self;
use mach2::vm::{mach_vm_deallocate, mach_vm_region_recurse};
use mach2::vm_prot::VM_PROT_EXECUTE;
use mach2::vm_region::vm_region_submap_info_64;
use mach2::vm_types::{mach_vm_address_t, mach_vm_size_t, natural_t};

use super::mem_common::{
    bytes, collect, file_name, gpu_items, images, jsc_stats, mb, mimalloc, ms, renderer, rust_items, rust_json, side,
    side_json, stickers, stylesheet_compiles, tree,
};
use super::mem_fold::{self, classify, stack_slice, tag, thread_group, Backing, Cat, Fold, Region};
use crate::json::Json;

// ---------------------------------------------------------------------------
// the SDK's layouts
// ---------------------------------------------------------------------------
//
// mach2 declares both `#[repr(C, packed(4))]`, as the headers' `#pragma
// pack(4)` does; the sizes are pinned here so a crate update that drifts from
// the SDK fails the build instead of reading the wrong field.
//
// `vm_region_submap_info_64` (mach/vm_region.h): protection, max_protection,
// inheritance, offset, user_tag, pages_resident, pages_shared_now_private,
// pages_swapped_out, pages_dirtied, ref_count, shadow_depth, external_pager,
// share_mode, is_submap, behavior, object_id, user_wired_count, flags,
// pages_reusable, object_id_full — 76 bytes, VM_REGION_SUBMAP_INFO_V2_COUNT_64
// = 19 natural_t.
const _: () = assert!(size_of::<vm_region_submap_info_64>() == 76);
// `task_vm_info` (mach/task_info.h, rev 7): virtual_size, region_count,
// page_size, resident_size(_peak), device(_peak), internal(_peak),
// external(_peak), reusable(_peak), purgeable_volatile_{pmap,resident,virtual},
// compressed(_peak, _lifetime), phys_footprint (rev 1), min/max_address
// (rev 2), 21 ledger_* (rev 3), limit_bytes_remaining (4), decompressions (5),
// ledger_swapins (6), ledger_tag_neural_nofootprint_{total,peak} (7) — 372
// bytes, TASK_VM_INFO_COUNT = 93 natural_t.
const _: () = assert!(size_of::<task_vm_info>() == 372);

extern "C" {
    /// mimalloc's options (`mi_option_get`); [`MI_OPTION_OS_TAG`] is the VM
    /// tag it puts on its OS memory.
    fn mi_option_get(option: c_int) -> c_long;
    /// libdyld (macOS 10.15+): where the dyld shared cache is mapped.
    fn _dyld_get_shared_cache_range(length: *mut usize) -> *const c_void;
}

/// `mi_option_os_tag` in mimalloc.h's `mi_option_e` (the 19th option).
const MI_OPTION_OS_TAG: c_int = 18;

/// The tag mimalloc tags its OS memory with in this process: its option, with
/// the same fallback as `unix_mmap_fd` (mimalloc `prim/unix/prim.c`).
fn mimalloc_tag() -> u32 {
    let t = unsafe { mi_option_get(MI_OPTION_OS_TAG) };
    if (100..=255).contains(&t) { t as u32 } else { 254 }
}

// ---------------------------------------------------------------------------
// the task's counters
// ---------------------------------------------------------------------------

struct Vm {
    footprint: f64,
    resident: f64,
    internal: f64,
    /// The task map's page size, which the region counts are in.
    page: u64,
    /// GPU memory charged to the task (graphics ledger, + its compressed
    /// part); 0 from a kernel older than rev 3.
    graphics: f64,
    json: Json,
}

fn vm_info() -> Vm {
    let mut info = task_vm_info::default();
    let mut count = (size_of::<task_vm_info>() / size_of::<natural_t>()) as mach_msg_type_number_t;
    let kr = unsafe {
        task_info(
            mach_task_self(),
            TASK_VM_INFO,
            (&mut info as *mut task_vm_info).cast(),
            &mut count,
        )
    };
    let page = (unsafe { libc::vm_page_size } as u64).max(4096);
    if kr != KERN_SUCCESS {
        return Vm { footprint: 0.0, resident: 0.0, internal: 0.0, page, graphics: 0.0, json: Json::Null };
    }
    // The kernel fills the newest revision `count` has room for and says how
    // much it filled: a field past that is not there.
    let has = |offset: usize| count as usize * size_of::<natural_t>() >= offset + 8;
    // Packed: every field is copied out before use.
    let (virtual_size, resident, resident_peak) = (info.virtual_size, info.resident_size, info.resident_size_peak);
    let (internal, external, compressed) = (info.internal, info.external, info.compressed);
    let (reusable, purgeable) = (info.reusable, info.purgeable_volatile_virtual);
    let page_size = info.page_size;
    let footprint = if has(offset_of!(task_vm_info, phys_footprint)) { info.phys_footprint } else { internal + compressed };
    let footprint_peak = if has(offset_of!(task_vm_info, ledger_phys_footprint_peak)) {
        Json::n((info.ledger_phys_footprint_peak as f64 / 1048576.0 * 10.0).round() / 10.0)
    } else {
        Json::Null
    };
    let (graphics, graphics_nofootprint) = if has(offset_of!(task_vm_info, ledger_tag_graphics_nofootprint_compressed)) {
        (
            (info.ledger_tag_graphics_footprint + info.ledger_tag_graphics_footprint_compressed).max(0) as f64,
            (info.ledger_tag_graphics_nofootprint + info.ledger_tag_graphics_nofootprint_compressed).max(0) as f64,
        )
    } else {
        (0.0, 0.0)
    };
    Vm {
        footprint: footprint as f64,
        resident: resident as f64,
        internal: internal as f64,
        page: if page_size > 0 { page_size as u64 } else { page },
        graphics,
        json: Json::obj(vec![
            // Activity Monitor's "Memory".
            ("footprintMB", mb(footprint as f64)),
            ("footprintPeakMB", footprint_peak),
            ("residentMB", mb(resident as f64)),
            ("residentPeakMB", mb(resident_peak as f64)),
            ("virtualMB", mb(virtual_size as f64)),
            ("internalMB", mb(internal as f64)),
            ("externalMB", mb(external as f64)),
            ("compressedMB", mb(compressed as f64)),
            ("reusableMB", mb(reusable as f64)),
            ("purgeableVolatileMB", mb(purgeable as f64)),
            ("graphicsFootprintMB", mb(graphics)),
            ("graphicsNoFootprintMB", mb(graphics_nofootprint)),
            ("pageSize", Json::n(page_size as f64)),
        ]),
    }
}

// ---------------------------------------------------------------------------
// threads
// ---------------------------------------------------------------------------

/// Every pthread's stack `(low, high, group)`, the threads per group, and how
/// many threads the task has.
struct Threads {
    stacks: Vec<(u64, u64, String)>,
    groups: HashMap<String, usize>,
    count: usize,
}

/// The task's threads. A thread that exits meanwhile is skipped: the pthread
/// calls validate the thread under libpthread's list lock and fail (ESRCH,
/// returned in place of the pointer / size) rather than read it.
fn threads() -> Threads {
    let task = unsafe { mach_task_self() };
    let mut list: thread_act_array_t = std::ptr::null_mut();
    let mut n: mach_msg_type_number_t = 0;
    if unsafe { task_threads(task, &mut list, &mut n) } != KERN_SUCCESS || list.is_null() {
        return Threads { stacks: Vec::new(), groups: HashMap::new(), count: 0 };
    }
    let ports: Vec<thread_act_t> = unsafe { std::slice::from_raw_parts(list, n as usize) }.to_vec();
    let mut stacks = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for port in &ports {
        let t = unsafe { libc::pthread_from_mach_thread_np(*port) };
        if t != 0 {
            let mut name = [0 as c_char; 64];
            let named = unsafe { libc::pthread_getname_np(t, name.as_mut_ptr(), name.len()) } == 0;
            let raw = if named { unsafe { CStr::from_ptr(name.as_ptr()) }.to_string_lossy().into_owned() } else { String::new() };
            let group = thread_group(&raw);
            let high = unsafe { libc::pthread_get_stackaddr_np(t) } as u64;
            let size = unsafe { libc::pthread_get_stacksize_np(t) } as u64;
            if high > 0x10000 && size >= 4096 && size <= high {
                stacks.push((high - size, high, group.clone()));
            }
            *counts.entry(group).or_default() += 1;
        }
        unsafe { mach_port_deallocate(task, *port) };
    }
    unsafe {
        mach_vm_deallocate(task, list as mach_vm_address_t, (n as usize * size_of::<thread_act_t>()) as mach_vm_size_t)
    };
    Threads { stacks, groups: counts, count: ports.len() }
}

// ---------------------------------------------------------------------------
// the address space
// ---------------------------------------------------------------------------

/// The file names of every image dyld has loaded (the exe, its dylibs and
/// frameworks): a file-backed region of one of them is an image, not a
/// mapped file.
fn image_names() -> HashSet<String> {
    let n = unsafe { _dyld_image_count() };
    (0..n)
        .filter_map(|i| {
            let p = unsafe { _dyld_get_image_name(i) };
            (!p.is_null()).then(|| file_name(&unsafe { CStr::from_ptr(p) }.to_string_lossy()))
        })
        .collect()
}

fn region_file(address: u64) -> Option<String> {
    let mut buf = [0u8; 1024];
    let n = unsafe { libc::proc_regionfilename(libc::getpid(), address, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| file_name(&String::from_utf8_lossy(&buf[..(n as usize).min(buf.len())])))
}

/// Every leaf region, in address order, the submaps (the dyld shared region)
/// recursed into.
fn regions() -> Vec<Region> {
    let task = unsafe { mach_task_self() };
    let (cache_lo, cache_hi) = {
        let mut len = 0usize;
        let p = unsafe { _dyld_get_shared_cache_range(&mut len) } as u64;
        (p, p.saturating_add(len as u64))
    };
    let images = image_names();
    let mut out = Vec::new();
    let mut address: mach_vm_address_t = 0;
    let mut depth: natural_t = 0;
    loop {
        let mut size: mach_vm_size_t = 0;
        let mut info = vm_region_submap_info_64::default();
        let mut count = (size_of::<vm_region_submap_info_64>() / size_of::<natural_t>()) as mach_msg_type_number_t;
        let kr = unsafe {
            mach_vm_region_recurse(
                task,
                &mut address,
                &mut size,
                &mut depth,
                (&mut info as *mut vm_region_submap_info_64).cast(),
                &mut count,
            )
        };
        if kr != KERN_SUCCESS {
            break;
        }
        if info.is_submap != 0 {
            depth += 1;
            continue;
        }
        // `pages_reusable` is V1 (count ≥ 17); an older kernel leaves it out.
        let reusable = if count as usize * size_of::<natural_t>() >= offset_of!(vm_region_submap_info_64, pages_reusable) + 4 {
            info.pages_reusable
        } else {
            0
        };
        let shared_cache = depth > 0 || (cache_lo != 0 && address >= cache_lo && address < cache_hi);
        let file_backed = info.external_pager != 0;
        let (backing, name) = if !file_backed {
            (Backing::Anonymous, None)
        } else if shared_cache {
            (Backing::Image, None)
        } else {
            let name = region_file(address);
            let image = info.user_tag == tag::DYLIB || name.as_ref().is_some_and(|n| images.contains(n));
            (if image { Backing::Image } else { Backing::File }, name)
        };
        out.push(Region {
            start: address,
            size,
            tag: info.user_tag,
            share_mode: info.share_mode,
            backing,
            shared_cache,
            exec: info.protection & VM_PROT_EXECUTE != 0,
            name,
            resident: u64::from(info.pages_resident),
            dirtied: u64::from(info.pages_dirtied),
            swapped: u64::from(info.pages_swapped_out),
            reusable: u64::from(reusable),
        });
        let next = address.saturating_add(size);
        if next <= address {
            break;
        }
        address = next;
    }
    out
}

// ---------------------------------------------------------------------------
// the malloc zones
// ---------------------------------------------------------------------------

struct Zones {
    in_use: f64,
    json: Json,
}

/// Every malloc zone's statistics summed (`malloc_zone_statistics(NULL)`):
/// the default zone Rust's `System` allocator and C/C++/Objective-C `malloc`
/// share, and the frameworks' own zones. mimalloc is not a zone here (no
/// override), so JSC's malloc is not in it.
fn zones() -> Zones {
    let mut s = libc::malloc_statistics_t { blocks_in_use: 0, size_in_use: 0, max_size_in_use: 0, size_allocated: 0 };
    unsafe { libc::malloc_zone_statistics(std::ptr::null_mut(), &mut s) };
    Zones {
        in_use: s.size_in_use as f64,
        json: Json::obj(vec![(
            "mallocZones",
            Json::obj(vec![
                ("inUseMB", mb(s.size_in_use as f64)),
                ("allocatedMB", mb(s.size_allocated as f64)),
                ("maxInUseMB", mb(s.max_size_in_use as f64)),
                ("blocks", Json::n(f64::from(s.blocks_in_use))),
            ]),
        )]),
    }
}

// ---------------------------------------------------------------------------
// the report
// ---------------------------------------------------------------------------

/// `addressSpace`: the walk's own view, in MB for `ctl mem`.
fn address_json(walk: &[Region], fold: &Fold, page: u64, threads: usize, stack_rows: &[(String, f64, f64)], regions: bool) -> Json {
    let page = page as f64;
    let virtual_size: u64 = walk.iter().map(|r| r.size).sum();
    let resident: u64 = walk.iter().map(|r| r.resident).sum();
    let images: HashSet<&str> = walk.iter().filter(|r| r.backing == Backing::Image).filter_map(|r| r.name.as_deref()).collect();
    let mut by_tag: HashMap<u32, [f64; 3]> = HashMap::new();
    for r in walk.iter().filter(|r| r.backing == Backing::Anonymous) {
        let t = by_tag.entry(r.tag).or_default();
        t[0] += r.charged_pages() as f64 * page;
        t[1] += r.resident as f64 * page;
        t[2] += r.size as f64;
    }
    let mut tags: Vec<(u32, [f64; 3])> = by_tag.into_iter().collect();
    tags.sort_by(|a, b| b.1[0].total_cmp(&a.1[0]));
    let modules: Vec<Json> = fold
        .slices_of(Cat::Images)
        .into_iter()
        .take(8)
        .map(|(name, b)| Json::obj(vec![("module", Json::s(name)), ("footprintMB", mb(b))]))
        .collect();
    let cat = |c: Cat| mb(fold.of(c));
    let mut o = vec![
        ("virtualMB", mb(virtual_size as f64)),
        ("residentMB", mb(resident as f64 * page)),
        ("regions", Json::n(walk.len() as f64)),
        ("modules", Json::n(images.len() as f64)),
        ("threads", Json::n(threads as f64)),
        ("imageModules", Json::arr(modules)),
        (
            "stacks",
            Json::arr(
                stack_rows
                    .iter()
                    .map(|(group, footprint, virtual_size)| {
                        Json::obj(vec![
                            ("threads", Json::s(group.clone())),
                            ("committedBytes", bytes(*footprint)),
                            ("reservedBytes", bytes(*virtual_size)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "footprintByCategoryMB",
            Json::obj(vec![
                ("images", cat(Cat::Images)),
                ("mapped", cat(Cat::Mapped)),
                ("jsc", cat(Cat::Jsc)),
                ("rust", cat(Cat::Rust)),
                ("graphics", cat(Cat::Graphics)),
                ("other", cat(Cat::Other)),
            ]),
        ),
        (
            // Anonymous memory by VM tag, the way `vmmap --summary` lists it.
            "byTag",
            Json::arr(
                tags.iter()
                    .take(16)
                    .map(|(t, [footprint, resident, virtual_size])| {
                        Json::obj(vec![
                            ("tag", Json::n(f64::from(*t))),
                            ("name", Json::s(mem_fold::tag_name(*t))),
                            ("footprintMB", mb(*footprint)),
                            ("residentMB", mb(*resident)),
                            ("virtualMB", mb(*virtual_size)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    if regions {
        let mut untagged: Vec<&Region> =
            walk.iter().filter(|r| r.backing == Backing::Anonymous && r.tag == 0 && r.charged_pages() > 0).collect();
        untagged.sort_by_key(|r| std::cmp::Reverse(r.charged_pages()));
        o.push((
            "largestUnattributed",
            Json::arr(
                untagged
                    .iter()
                    .take(16)
                    .map(|r| {
                        Json::obj(vec![
                            ("base", Json::s(format!("{:#x}", r.start))),
                            ("committedBytes", bytes(r.charged_pages() as f64 * page)),
                            ("reservedBytes", bytes(r.size as f64)),
                            ("executable", Json::Bool(r.exec)),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    Json::obj(o)
}

/// The whole report; see `mem.rs` for `gc` and `code`. `regions` adds the
/// resident bytes per category and the largest untagged regions.
pub(crate) fn report(gc: bool, code: bool, regions: bool, window: &mut Window, cx: &mut App) -> Json {
    let gc = gc || code;
    let before = if gc { Some((vm_info(), jsc_stats())) } else { None };
    let gc_ms = gc.then(|| collect(code));
    let t = std::time::Instant::now();
    let vm = vm_info();
    let threads = threads();
    let walk = self::regions();
    let mi_tag = mimalloc_tag();
    let mut fold = Fold::default();
    let mut stack_rows: HashMap<String, (f64, f64)> = HashMap::new();
    for r in &walk {
        let (cat, mut slice) = classify(r, mi_tag);
        if r.backing == Backing::Anonymous && r.tag == tag::STACK {
            slice = stack_slice(r.start, &threads.stacks, &threads.groups);
            let row = stack_rows.entry(slice.clone()).or_default();
            row.0 += (r.charged_pages() * vm.page) as f64;
            row.1 += r.size as f64;
        }
        fold.add(r, cat, slice, vm.page);
    }
    let mut stack_rows: Vec<(String, f64, f64)> = stack_rows.into_iter().map(|(k, (f, v))| (k, f, v)).collect();
    stack_rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    let zones = zones();
    let mi = mimalloc();
    let jsc = jsc_stats();
    let renderer = renderer(window);
    let images = images(cx);
    let (tree_json, tree_bytes) = tree();
    let side = side();
    let rust = crate::heapcount::snapshot();
    let rust_live = rust.map(|(live, ..)| live as f64);
    let gpu_named = renderer.atlas.iter().sum::<f64>() + renderer.targets;
    let breakdown = mem_fold::breakdown(&mem_fold::Inputs {
        fold: &fold,
        footprint: vm.footprint,
        resident_size: vm.resident,
        internal: vm.internal,
        graphics_ledger: vm.graphics,
        malloc_in_use: zones.in_use,
        mi_live: mi.live,
        mi_committed: mi.committed,
        jsc: [jsc.size, jsc.capacity, jsc.extra],
        rust_live,
        rust_items: rust_live.map(|live| rust_items(live, tree_bytes, &images, &side)).unwrap_or_default(),
        gpu_items: gpu_items(&renderer),
        gpu_named,
        with_resident: regions,
    });
    // What the allocators we can see account for, against the footprint.
    let explained = fold.of(Cat::Rust) + fold.of(Cat::Jsc) + fold.of(Cat::Graphics);
    let mut o = vec![
        ("breakdown", breakdown),
        ("process", vm.json),
        ("addressSpace", address_json(&walk, &fold, vm.page, threads.count, &stack_rows, regions)),
        ("heaps", zones.json),
        ("rustHeap", rust_json(rust)),
        ("mimalloc", mi.json),
        ("jsc", jsc.json),
        (
            // One row: Apple GPUs share the system's memory, so there is no
            // adapter's VRAM to report, only what is charged to this task.
            "gpu",
            Json::arr(vec![Json::obj(vec![
                ("adapter", Json::s("unified memory")),
                ("graphicsFootprintMB", mb(vm.graphics)),
                ("mappedMB", mb(fold.of(Cat::Graphics))),
                ("mimallocTag", Json::n(f64::from(mi_tag))),
            ])]),
        ),
        ("renderer", renderer.json),
        ("images", images.json),
        ("tree", tree_json),
        ("side", side_json(&side)),
        ("stickers", stickers()),
        ("httpCache", super::net::cache_json()),
        ("stylesheetCompiles", stylesheet_compiles()),
        (
            "explained",
            Json::obj(vec![
                ("footprintMB", mb(vm.footprint)),
                ("mallocZonesMB", mb(fold.of(Cat::Rust))),
                ("mimallocMB", mb(mi.committed)),
                ("jscRegionsMB", mb(fold.of(Cat::Jsc))),
                ("graphicsMB", mb(fold.of(Cat::Graphics))),
                ("sumMB", mb(explained)),
                ("unexplainedMB", mb(vm.footprint - explained)),
                ("gpuProcessMB", mb(vm.graphics)),
            ]),
        ),
        ("collectMs", Json::n(ms(t))),
    ];
    if let (Some(took), Some((before_vm, before_jsc))) = (gc_ms, before) {
        o.push((
            "gc",
            Json::obj(vec![
                ("ms", Json::n(took)),
                (
                    "before",
                    Json::obj(vec![("process", before_vm.json), ("jsc", before_jsc.json)]),
                ),
                // The footprint is what macOS charges; "private" keeps the
                // field the inspector already reads.
                ("freedPrivateBytes", bytes(before_vm.footprint - vm.footprint)),
                ("freedJscHeapBytes", bytes(before_jsc.size - jsc.size)),
            ]),
        ));
    }
    Json::obj(o)
}
