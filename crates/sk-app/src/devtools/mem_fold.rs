//! The macOS `mem` fold, free of Mach calls so it is tested on every OS; `mem_mac.rs` fills [`Region`]s.
//!
//! Categories sum to the task's physical footprint (Activity Monitor's "Memory"): each region
//! contributes the pages the kernel charges for it ([`Region::charged_pages`]), and what no region
//! holds is its own row. If the estimate exceeds the kernel's figure (shared pages are charged to
//! one process only), the total is the estimate and an `info` row says by how much.

use std::collections::HashMap;

use super::mem_common::{bytes, category, info, item, with_resident};
use crate::json::Json;

// `share_mode` (mach/vm_region.h).
#[cfg(test)]
const SM_COW: u8 = 1;
#[cfg(test)]
const SM_PRIVATE: u8 = 2;
pub(super) const SM_EMPTY: u8 = 3;
pub(super) const SM_SHARED: u8 = 4;
pub(super) const SM_TRUESHARED: u8 = 5;
pub(super) const SM_SHARED_ALIASED: u8 = 7;

/// The `VM_MEMORY_*` tags the fold names.
pub(super) mod tag {
    pub const MALLOC: u32 = 1;
    pub const MALLOC_SMALL: u32 = 2;
    pub const MALLOC_LARGE: u32 = 3;
    pub const MALLOC_HUGE: u32 = 4;
    pub const SBRK: u32 = 5;
    pub const REALLOC: u32 = 6;
    pub const MALLOC_TINY: u32 = 7;
    pub const MALLOC_LARGE_REUSABLE: u32 = 8;
    pub const MALLOC_LARGE_REUSED: u32 = 9;
    pub const MALLOC_NANO: u32 = 11;
    pub const MALLOC_MEDIUM: u32 = 12;
    pub const MALLOC_PROB_GUARD: u32 = 13;
    pub const IOKIT: u32 = 21;
    pub const STACK: u32 = 30;
    pub const GUARD: u32 = 31;
    pub const DYLIB: u32 = 33;
    pub const COREGRAPHICS: u32 = 42;
    pub const LAYERKIT: u32 = 51;
    pub const CGIMAGE: u32 = 52;
    pub const COREGRAPHICS_DATA: u32 = 54;
    pub const COREGRAPHICS_SHARED: u32 = 55;
    pub const COREGRAPHICS_FRAMEBUFFERS: u32 = 56;
    pub const COREGRAPHICS_BACKINGSTORES: u32 = 57;
    pub const COREGRAPHICS_XALLOC: u32 = 58;
    pub const JAVASCRIPT_CORE: u32 = 63;
    pub const JAVASCRIPT_JIT_EXECUTABLE_ALLOCATOR: u32 = 64;
    pub const JAVASCRIPT_JIT_REGISTER_FILE: u32 = 65;
    pub const GLSL: u32 = 66;
    pub const OPENCL: u32 = 67;
    pub const COREIMAGE: u32 = 68;
    pub const IMAGEIO: u32 = 70;
    pub const IOSURFACE: u32 = 88;
    pub const IOACCELERATOR: u32 = 100;
    /// mimalloc's default `mi_option_os_tag`; `mem_mac.rs` asks mimalloc instead.
    #[cfg(test)]
    pub const MIMALLOC_DEFAULT: u32 = 240;
}

/// What a region maps.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Backing {
    #[default]
    Anonymous,
    /// A Mach-O image dyld loaded, or the dyld shared cache.
    Image,
    /// Any other file.
    File,
}

/// One leaf region of the walk; the counts are in pages.
#[derive(Clone, Debug, Default)]
pub(super) struct Region {
    // Read by `mem_mac.rs`, not by the fold.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub start: u64,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub size: u64,
    pub tag: u32,
    pub share_mode: u8,
    pub backing: Backing,
    pub shared_cache: bool,
    pub exec: bool,
    /// File name (not path) of a file-backed region outside the shared cache.
    pub name: Option<String>,
    pub resident: u64,
    pub dirtied: u64,
    pub swapped: u64,
    pub reusable: u64,
}

impl Region {
    /// A file's clean pages are the file cache's (only dirty or compressed ones count); anonymous
    /// memory counts while resident or compressed, minus `MADV_FREE_REUSABLE` pages.
    pub fn charged_pages(&self) -> u64 {
        match self.share_mode {
            SM_EMPTY => 0,
            _ if self.backing != Backing::Anonymous => self.dirtied + self.swapped,
            _ => self.resident.saturating_sub(self.reusable) + self.swapped,
        }
    }

    pub fn shared(&self) -> bool {
        matches!(self.share_mode, SM_SHARED | SM_TRUESHARED | SM_SHARED_ALIASED)
    }
}

/// In the order the panel draws them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Cat {
    Images,
    Mapped,
    Jsc,
    Rust,
    Graphics,
    Other,
}

pub(super) const MIMALLOC: &str = "mimalloc";
pub(super) const STACKS: &str = "thread stacks";

/// Digit runs folded to `#`, so a pool's workers group into one row.
pub(super) fn thread_group(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
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

/// The group of the thread whose `[low, high)` stack contains the region, with its thread count.
pub(super) fn stack_slice(start: u64, stacks: &[(u64, u64, String)], counts: &HashMap<String, usize>) -> String {
    match stacks.iter().find(|(low, high, _)| *low <= start && start < *high) {
        Some((_, _, g)) => format!("thread stacks: {g} ×{}", counts.get(g).copied().unwrap_or(1)),
        None => format!("{STACKS}: no live pthread (exited, or not a pthread)"),
    }
}

/// The `vmmap` name of a tag that is not a category of its own.
pub(super) fn tag_name(t: u32) -> String {
    let s = match t {
        0 => "untagged anonymous memory (mmap / vm_allocate without a tag)",
        tag::SBRK => "sbrk",
        10 => "analysis tool",
        20 => "Mach messages",
        tag::GUARD => "guard pages",
        32 | 35 => "pmap",
        tag::DYLIB => "dylib",
        34 => "Objective-C dispatchers",
        40 => "AppKit",
        41 => "Foundation",
        43 => "CoreServices",
        44 => "Java",
        45 | 46 => "CoreData",
        50 => "ATS (font support)",
        53 => "TCMalloc",
        60 => "dyld",
        61 => "dyld private memory (dyld malloc)",
        62 => "SQLite page cache",
        69 => "WebCore purgeable buffers",
        71 => "CoreProfile",
        72 => "assetsd",
        73 => "os_alloc_once",
        74 => "libdispatch",
        75 => "Accelerate",
        76 => "CoreUI",
        77 => "CoreUI image files",
        78 => "activity tracing (genealogy)",
        79 => "RawCamera",
        80 => "corpse info",
        81 => "ASL",
        82 => "Swift runtime",
        83 => "Swift metadata",
        84 => "DHMM",
        86 => "SceneKit",
        87 => "Skywalk networking",
        89 => "libnetwork",
        90 => "Audio",
        91 => "video bitstream",
        92..=96 | 101 | 106 => "CoreMedia",
        97 => "QuickLook thumbnails",
        98 => "Accounts",
        99 => "sanitizer",
        102 => "EAR decoder",
        103 => "CoreUI cached image data",
        104 => "ColorSync",
        105 => "backtrace info",
        230 | 231 => "Rosetta",
        _ => return format!("VM tag {t}"),
    };
    s.to_string()
}

/// The region's category and slice. `mi_tag` is mimalloc's OS-memory tag.
pub(super) fn classify(r: &Region, mi_tag: u32) -> (Cat, String) {
    match r.backing {
        Backing::Image if r.shared_cache => (Cat::Images, "dyld shared cache (system libraries' written __DATA)".into()),
        Backing::Image => (Cat::Images, r.name.clone().unwrap_or_else(|| "(unnamed image)".into())),
        Backing::File => (Cat::Mapped, r.name.clone().unwrap_or_else(|| "(unnamed file)".into())),
        Backing::Anonymous => {
            let t = r.tag;
            let s = |s: &str| s.to_string();
            match t {
                _ if t == mi_tag => (Cat::Jsc, s(MIMALLOC)),
                tag::JAVASCRIPT_CORE => (Cat::Jsc, s("JavaScriptCore VM regions (tag 63: Gigacage, structure heap)")),
                tag::JAVASCRIPT_JIT_EXECUTABLE_ALLOCATOR => (Cat::Jsc, s("JIT code (executable pages)")),
                tag::JAVASCRIPT_JIT_REGISTER_FILE => (Cat::Jsc, s("JS stack / register file (tag 65)")),
                0 if r.exec => (Cat::Jsc, s("JIT code (executable pages)")),
                tag::MALLOC => (Cat::Rust, s("malloc: zone metadata")),
                tag::MALLOC_TINY => (Cat::Rust, s("malloc: tiny")),
                tag::MALLOC_SMALL => (Cat::Rust, s("malloc: small")),
                tag::MALLOC_MEDIUM => (Cat::Rust, s("malloc: medium")),
                tag::MALLOC_LARGE | tag::MALLOC_LARGE_REUSABLE | tag::MALLOC_LARGE_REUSED | tag::MALLOC_HUGE | tag::REALLOC => {
                    (Cat::Rust, s("malloc: large"))
                }
                tag::MALLOC_NANO => (Cat::Rust, s("malloc: nano")),
                tag::MALLOC_PROB_GUARD => (Cat::Rust, s("malloc: probabilistic guard")),
                tag::IOACCELERATOR => (Cat::Graphics, s("IOAccelerator (Metal buffers and textures mapped here)")),
                tag::IOSURFACE => (Cat::Graphics, s("IOSurface (window and layer backing)")),
                tag::IOKIT => (Cat::Graphics, s("IOKit mappings")),
                tag::LAYERKIT => (Cat::Graphics, s("CoreAnimation")),
                tag::COREGRAPHICS
                | tag::COREGRAPHICS_DATA
                | tag::COREGRAPHICS_SHARED
                | tag::COREGRAPHICS_FRAMEBUFFERS
                | tag::COREGRAPHICS_BACKINGSTORES
                | tag::COREGRAPHICS_XALLOC => (Cat::Graphics, s("CoreGraphics")),
                tag::CGIMAGE => (Cat::Graphics, s("CGImage")),
                tag::IMAGEIO => (Cat::Graphics, s("ImageIO")),
                tag::COREIMAGE => (Cat::Graphics, s("CoreImage")),
                tag::GLSL | tag::OPENCL => (Cat::Graphics, s("GLSL / OpenCL")),
                tag::STACK => (Cat::Other, s(STACKS)),
                0 if r.shared() => (Cat::Mapped, s("shared memory (anonymous, shared with another process)")),
                _ => (Cat::Other, tag_name(t)),
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Fold {
    pub bytes: HashMap<Cat, f64>,
    pub slices: HashMap<(Cat, String), f64>,
    /// `[private, shared]` resident bytes per category.
    pub resident: HashMap<Cat, [f64; 2]>,
    pub attributed: f64,
}

impl Fold {
    pub fn add(&mut self, r: &Region, cat: Cat, slice: String, page: u64) {
        let charged = (r.charged_pages() * page) as f64;
        self.attributed += charged;
        *self.bytes.entry(cat).or_default() += charged;
        let res = self.resident.entry(cat).or_default();
        res[usize::from(r.shared())] += (r.resident * page) as f64;
        if charged > 0.0 {
            *self.slices.entry((cat, slice)).or_default() += charged;
        }
    }

    pub fn of(&self, cat: Cat) -> f64 {
        self.bytes.get(&cat).copied().unwrap_or(0.0)
    }

    /// Biggest first.
    pub fn slices_of(&self, cat: Cat) -> Vec<(String, f64)> {
        let mut v: Vec<(String, f64)> =
            self.slices.iter().filter(|((c, _), _)| *c == cat).map(|((_, s), b)| (s.clone(), *b)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// The biggest `n` slices as rows, then the rest as one (`what` names them).
    fn rows(&self, cat: Cat, n: usize, what: &str, skip: &[&str]) -> Vec<Json> {
        let all: Vec<(String, f64)> = self.slices_of(cat).into_iter().filter(|(s, _)| !skip.contains(&s.as_str())).collect();
        let mut rows: Vec<Json> = all.iter().take(n).map(|(s, b)| item(s.clone(), *b)).collect();
        if all.len() > n {
            let rest: f64 = all.iter().skip(n).map(|(_, b)| *b).sum();
            rows.push(item(format!("{} other {what}", all.len() - n), rest));
        }
        rows
    }
}

/// Everything [`breakdown`] needs besides the fold, in bytes.
pub(super) struct Inputs<'a> {
    pub fold: &'a Fold,
    /// `phys_footprint`.
    pub footprint: f64,
    pub resident_size: f64,
    /// Anonymous memory in RAM.
    pub internal: f64,
    /// GPU memory charged to this task; 0 from a kernel that does not report it.
    pub graphics_ledger: f64,
    /// Bytes in use in every malloc zone.
    pub malloc_in_use: f64,
    pub mi_live: f64,
    pub mi_committed: f64,
    /// JSC's own count: GC heap size, capacity, extra memory.
    pub jsc: [f64; 3],
    pub rust_live: Option<f64>,
    /// `mem_common::rust_items` when the Rust heap is counted.
    pub rust_items: Vec<Json>,
    pub gpu_items: Vec<Json>,
    pub gpu_named: f64,
    /// Add `residentBytes` per category.
    pub with_resident: bool,
}

/// Bytes; sums to the physical footprint, or the region estimate where that is larger.
pub(super) fn breakdown(i: &Inputs) -> Json {
    let f = i.fold;
    let residual = i.footprint - f.attributed;
    let unmapped = residual.max(0.0);
    let gpu_unmapped = unmapped.min(i.graphics_ledger.max(0.0));
    let rest_unmapped = unmapped - gpu_unmapped;
    let total = f.attributed + unmapped;

    let mut image_items = f.rows(Cat::Images, 10, "modules", &[]);
    image_items.push(info(
        "resident image pages, clean and dirty (shared: not in the footprint)",
        f.resident.get(&Cat::Images).map_or(0.0, |r| r[0] + r[1]),
    ));
    let mapped_items = f.rows(Cat::Mapped, 8, "files", &[]);

    let mi_pages = f.slices.get(&(Cat::Jsc, MIMALLOC.to_string())).copied().unwrap_or(0.0);
    let mi_live = i.mi_live.min(mi_pages);
    let mut jsc_items = vec![
        item(
            "mimalloc: live allocations (GC heap blocks, strings, bytecode, the bundle's source)",
            mi_live,
        ),
        item("mimalloc: pages in use, not allocated (not yet purged)", mi_pages - mi_live),
    ];
    jsc_items.extend(f.rows(Cat::Jsc, 8, "regions", &[MIMALLOC]));
    jsc_items.push(info("by mimalloc's own count: committed", i.mi_committed));
    jsc_items.push(info("by JSC's own count: GC heap in use", i.jsc[0]));
    jsc_items.push(info("by JSC's own count: GC heap capacity", i.jsc[1]));
    jsc_items.push(info(
        "by JSC's own count: extra memory (ArrayBuffers, external strings)",
        i.jsc[2],
    ));

    let heap = f.of(Cat::Rust);
    let mut rust_items = Vec::new();
    match i.rust_live {
        Some(live) => {
            rust_items.extend(i.rust_items.iter().cloned());
            rust_items.push(item(
                "C/C++/Objective-C malloc in the same zones (in use − Rust live)",
                i.malloc_in_use - live,
            ));
        }
        None => rust_items.push(item(
            "in use: Rust + C/C++/Objective-C malloc (Rust heap counting is off)",
            i.malloc_in_use,
        )),
    }
    rust_items.push(item(
        "malloc slack (charged pages not in use: free blocks, zone overhead)",
        heap - i.malloc_in_use,
    ));
    for (s, b) in f.slices_of(Cat::Rust) {
        rust_items.push(info(format!("by region: {s}"), b));
    }

    let mut graphics_items = f.rows(Cat::Graphics, 8, "kinds", &[]);
    if gpu_unmapped > 0.0 {
        graphics_items.push(item(
            "GPU memory charged to this process, not mapped into it (graphics ledger)",
            gpu_unmapped,
        ));
    }
    if i.graphics_ledger > 0.0 {
        graphics_items.push(info("graphics ledger: all GPU memory charged to this process", i.graphics_ledger));
    }
    let graphics = f.of(Cat::Graphics) + gpu_unmapped;

    let mut other_items = f.rows(Cat::Other, 10, "kinds", &[]);
    if rest_unmapped > 0.0 {
        other_items.push(item(
            "not in any region: page tables, purgeable and ledger adjustments",
            rest_unmapped,
        ));
    }
    if residual < 0.0 {
        other_items.push(info(
            "the region walk counts this much more than the kernel's footprint (pages shared with another process)",
            -residual,
        ));
    }
    let other = f.of(Cat::Other) + rest_unmapped;

    let res = |c: Cat| i.with_resident.then(|| f.resident.get(&c).copied().unwrap_or([0.0, 0.0]));
    let cats = vec![
        with_resident(
            category(
                "images",
                "Loaded images (exe + dylibs)",
                f.of(Cat::Images),
                "written pages of loaded Mach-Os and the dyld shared cache; clean pages are the file cache's",
                image_items,
            ),
            res(Cat::Images),
        ),
        with_resident(
            category(
                "mapped",
                "Mapped files and shared memory",
                f.of(Cat::Mapped),
                "written pages of mapped files; anonymous memory shared with another process",
                mapped_items,
            ),
            res(Cat::Mapped),
        ),
        with_resident(
            category("jsc", "JSC", f.of(Cat::Jsc), "mimalloc (JSC's malloc) + JSC's own VM regions + JIT code", jsc_items),
            res(Cat::Jsc),
        ),
        with_resident(
            category(
                "rust",
                "Native heap (malloc zones)",
                heap,
                "libmalloc: Rust's allocator, C/C++ and Objective-C malloc, the system frameworks",
                rust_items,
            ),
            res(Cat::Rust),
        ),
        with_resident(
            category(
                "graphics",
                "Graphics (Metal, IOSurface, CoreAnimation)",
                graphics,
                "unified memory: GPU allocations are this process's pages",
                graphics_items,
            ),
            res(Cat::Graphics),
        ),
        with_resident(
            category(
                "other",
                "Other",
                other,
                "thread stacks, system frameworks' tagged memory, untagged memory, and what no region holds",
                other_items,
            ),
            res(Cat::Other),
        ),
    ];
    let mut gpu_rows = i.gpu_items.clone();
    gpu_rows.push(item(
        "rest (Metal heaps, IOSurfaces, the driver)",
        graphics - i.gpu_named,
    ));
    Json::obj(vec![
        (
            "basis",
            Json::s(
                "physical footprint (Activity Monitor's Memory): each VM region's dirty and compressed pages, by tag; what no region holds is its own row",
            ),
        ),
        ("totalLabel", Json::s("footprint")),
        ("totalBytes", bytes(total)),
        ("privateBytes", bytes(i.footprint)),
        ("workingSetBytes", bytes(i.resident_size)),
        ("privateWorkingSetBytes", bytes(i.internal)),
        ("rustCounting", Json::Bool(i.rust_live.is_some())),
        ("categories", Json::arr(cats)),
        (
            "gpu",
            Json::obj(vec![
                ("dedicatedBytes", bytes(0.0)),
                ("sharedBytes", bytes(graphics)),
                ("inBar", Json::Bool(true)),
                ("items", Json::arr(gpu_rows)),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: u64 = 16384;

    fn anon(tag: u32, resident: u64) -> Region {
        Region { tag, share_mode: SM_PRIVATE, resident, size: resident * PAGE, ..Default::default() }
    }

    #[test]
    fn charged_pages_follow_the_backing() {
        let mut r = anon(tag::MALLOC_TINY, 10);
        r.swapped = 3;
        r.reusable = 4;
        assert_eq!(r.charged_pages(), 10 - 4 + 3, "anonymous: resident − reusable + compressed");
        r.share_mode = SM_EMPTY;
        assert_eq!(r.charged_pages(), 0, "nothing behind it");
        let file = Region { backing: Backing::Image, share_mode: SM_COW, resident: 50, dirtied: 2, swapped: 1, ..Default::default() };
        assert_eq!(file.charged_pages(), 3, "a file's clean pages are the file cache's");
        let more = Region { reusable: 20, ..anon(0, 5) };
        assert_eq!(more.charged_pages(), 0, "reusable never underflows");
    }

    #[test]
    fn tags_land_in_the_panel_categories() {
        let mi = tag::MIMALLOC_DEFAULT;
        let cat = |r: Region| classify(&r, mi).0;
        assert_eq!(cat(anon(mi, 1)), Cat::Jsc);
        assert_eq!(classify(&anon(mi, 1), mi).1, MIMALLOC);
        // MIMALLOC_OS_TAG=250: 240 is then an ordinary tag.
        assert_eq!(classify(&anon(250, 1), 250).0, Cat::Jsc);
        assert_eq!(classify(&anon(240, 1), 250).0, Cat::Other);
        for t in [tag::JAVASCRIPT_CORE, tag::JAVASCRIPT_JIT_EXECUTABLE_ALLOCATOR, tag::JAVASCRIPT_JIT_REGISTER_FILE] {
            assert_eq!(cat(anon(t, 1)), Cat::Jsc, "tag {t}");
        }
        assert_eq!(cat(Region { exec: true, ..anon(0, 1) }), Cat::Jsc, "untagged executable: JIT");
        for t in [1, 2, 3, 4, 6, 7, 8, 9, 11, 12, 13] {
            assert_eq!(cat(anon(t, 1)), Cat::Rust, "malloc tag {t}");
        }
        for t in [tag::IOACCELERATOR, tag::IOSURFACE, tag::IOKIT, tag::LAYERKIT, tag::CGIMAGE, 54, 57] {
            assert_eq!(cat(anon(t, 1)), Cat::Graphics, "tag {t}");
        }
        assert_eq!(classify(&anon(tag::STACK, 1), mi), (Cat::Other, STACKS.to_string()));
        assert_eq!(cat(anon(0, 1)), Cat::Other);
        assert_eq!(cat(Region { share_mode: SM_SHARED, ..anon(0, 1) }), Cat::Mapped, "shared anonymous memory");
        assert_eq!(cat(Region { share_mode: SM_TRUESHARED, ..anon(tag::IOACCELERATOR, 1) }), Cat::Graphics, "the tag wins");
        let dylib = Region { backing: Backing::Image, name: Some("libvn.dylib".into()), ..Default::default() };
        assert_eq!(classify(&dylib, mi), (Cat::Images, "libvn.dylib".to_string()));
        let cache = Region { backing: Backing::Image, shared_cache: true, ..Default::default() };
        assert_eq!(cat(cache), Cat::Images);
        let font = Region { backing: Backing::File, name: Some("SFNS.ttf".into()), ..Default::default() };
        assert_eq!(classify(&font, mi), (Cat::Mapped, "SFNS.ttf".to_string()));
    }

    #[test]
    fn stacks_group_by_thread_name() {
        assert_eq!(thread_group("tokio-runtime-worker-12"), "tokio-runtime-worker-#");
        assert_eq!(thread_group(" JSC Heap Collector Thread 3 "), "JSC Heap Collector Thread #");
        assert_eq!(thread_group(""), "(unnamed)");
        let stacks = vec![(0x1000, 0x9000, "main".to_string()), (0x10000, 0x20000, "worker #".to_string())];
        let counts = HashMap::from([("main".to_string(), 1), ("worker #".to_string(), 4)]);
        assert_eq!(stack_slice(0x1000, &stacks, &counts), "thread stacks: main ×1");
        assert_eq!(stack_slice(0x1ffff, &stacks, &counts), "thread stacks: worker # ×4");
        assert!(stack_slice(0x9000, &stacks, &counts).starts_with("thread stacks: no live pthread"));
    }

    #[test]
    fn tag_names() {
        assert_eq!(tag_name(60), "dyld");
        assert_eq!(tag_name(62), "SQLite page cache");
        assert_eq!(tag_name(199), "VM tag 199");
    }

    fn inputs<'a>(fold: &'a Fold, footprint: f64, graphics_ledger: f64) -> Inputs<'a> {
        Inputs {
            fold,
            footprint,
            resident_size: footprint * 2.0,
            internal: footprint / 2.0,
            graphics_ledger,
            malloc_in_use: 3.0 * PAGE as f64,
            mi_live: 1.0 * PAGE as f64,
            mi_committed: 4.0 * PAGE as f64,
            jsc: [1.0, 2.0, 3.0],
            rust_live: Some(PAGE as f64),
            rust_items: vec![item("rest of the Rust heap", PAGE as f64)],
            gpu_items: vec![item("sprite atlas", 1.0)],
            gpu_named: 1.0,
            with_resident: true,
        }
    }

    fn sample() -> Fold {
        let mut fold = Fold::default();
        let regions = [
            anon(tag::MIMALLOC_DEFAULT, 4),
            anon(tag::JAVASCRIPT_JIT_EXECUTABLE_ALLOCATOR, 1),
            anon(tag::MALLOC_TINY, 2),
            anon(tag::MALLOC_SMALL, 3),
            anon(tag::IOACCELERATOR, 5),
            anon(tag::STACK, 1),
            anon(60, 1),
            Region { backing: Backing::Image, name: Some("sk-app".into()), share_mode: SM_COW, resident: 9, dirtied: 2, ..Default::default() },
            Region { backing: Backing::File, name: Some("icu.dat".into()), share_mode: SM_PRIVATE, resident: 7, ..Default::default() },
        ];
        for r in &regions {
            let (cat, slice) = classify(r, tag::MIMALLOC_DEFAULT);
            fold.add(r, cat, slice, PAGE);
        }
        fold
    }

    fn cats(b: &Json) -> Vec<(String, f64)> {
        b.get("categories")
            .and_then(Json::as_arr)
            .unwrap()
            .iter()
            .map(|c| (c.get("key").and_then(Json::as_str).unwrap().to_string(), c.get("bytes").and_then(Json::as_f64).unwrap()))
            .collect()
    }

    fn slices(b: &Json, key: &str) -> Vec<(String, f64, bool)> {
        let c = b.get("categories").and_then(Json::as_arr).unwrap().iter().find(|c| c.get("key").and_then(Json::as_str) == Some(key)).unwrap();
        c.get("items")
            .and_then(Json::as_arr)
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i.get("label").and_then(Json::as_str).unwrap().to_string(),
                    i.get("bytes").and_then(Json::as_f64).unwrap(),
                    i.get("info").and_then(Json::as_bool).unwrap_or(false),
                )
            })
            .collect()
    }

    #[test]
    fn categories_sum_to_the_footprint() {
        let fold = sample();
        let p = PAGE as f64;
        // 4+1+2+3+5+1+1 anonymous pages, 2 dirty image pages.
        assert_eq!(fold.attributed, 19.0 * p);
        // The kernel says 25 pages: 6 no region holds, 4 of them GPU memory.
        let b = breakdown(&inputs(&fold, 25.0 * p, 4.0 * p));
        let c = cats(&b);
        assert_eq!(
            c.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["images", "mapped", "jsc", "rust", "graphics", "other"]
        );
        let total = b.get("totalBytes").and_then(Json::as_f64).unwrap();
        assert_eq!(total, 25.0 * p);
        assert_eq!(c.iter().map(|(_, v)| v).sum::<f64>(), total);
        assert_eq!(c[4].1, 9.0 * p, "graphics: 5 mapped IOAccelerator pages + 4 from the ledger");
        assert_eq!(c[5].1, 4.0 * p, "other: a stack page, a dyld page, 2 pages of page tables");
        assert_eq!(c[1].1, 0.0, "a clean mapped file costs no footprint");
        // Every category's slices add up to it.
        for (key, bytes) in &c {
            let sum: f64 = slices(&b, key).iter().filter(|s| !s.2).map(|s| s.1.max(0.0)).sum();
            assert_eq!(sum, *bytes, "{key}");
        }
        let jsc = slices(&b, "jsc");
        assert_eq!(jsc[0].1, p, "mimalloc live");
        assert_eq!(jsc[1].1, 3.0 * p, "mimalloc pages not allocated");
        let rust = slices(&b, "rust");
        assert!(rust.iter().any(|s| s.0 == "by region: malloc: small" && s.1 == 3.0 * p && s.2));
        let gpu = b.get("gpu").unwrap();
        assert_eq!(gpu.get("sharedBytes").and_then(Json::as_f64), Some(9.0 * p));
        assert_eq!(gpu.get("inBar").and_then(Json::as_bool), Some(true));
        let images = b.get("categories").and_then(Json::as_arr).unwrap()[0].clone();
        assert_eq!(images.get("residentBytes").and_then(Json::as_f64), Some(9.0 * p));
    }

    #[test]
    fn an_estimate_above_the_footprint_is_the_total_and_says_so() {
        let fold = sample();
        let p = PAGE as f64;
        let b = breakdown(&inputs(&fold, 15.0 * p, 8.0 * p));
        let c = cats(&b);
        let total = b.get("totalBytes").and_then(Json::as_f64).unwrap();
        assert_eq!(total, 19.0 * p);
        assert_eq!(c.iter().map(|(_, v)| v).sum::<f64>(), total);
        assert_eq!(b.get("privateBytes").and_then(Json::as_f64), Some(15.0 * p), "the kernel's figure is kept");
        let other = slices(&b, "other");
        assert!(other.iter().any(|s| s.2 && s.1 == 4.0 * p), "an info row names the overshoot");
        assert!(!slices(&b, "graphics").iter().any(|s| !s.2 && s.0.contains("ledger")), "no unmapped GPU slice");
    }

    #[test]
    fn without_regions_there_is_no_resident_column() {
        let fold = sample();
        let mut i = inputs(&fold, 19.0 * PAGE as f64, 0.0);
        i.with_resident = false;
        let b = breakdown(&i);
        let first = &b.get("categories").and_then(Json::as_arr).unwrap()[0];
        assert!(first.get("residentBytes").is_none());
    }

    #[test]
    fn many_slices_fold_into_a_rest_row() {
        let mut fold = Fold::default();
        for t in 200..215u32 {
            let r = anon(t, u64::from(t - 199));
            let (cat, slice) = classify(&r, tag::MIMALLOC_DEFAULT);
            fold.add(&r, cat, slice, PAGE);
        }
        let b = breakdown(&inputs(&fold, fold.attributed, 0.0));
        let other = slices(&b, "other");
        assert_eq!(other.len(), 11);
        assert_eq!(other[0].0, "VM tag 214", "biggest first");
        assert_eq!(other[10].0, "5 other kinds");
        assert_eq!(other[10].1, (1 + 2 + 3 + 4 + 5) as f64 * PAGE as f64);
    }
}
