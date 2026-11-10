//! What `mem` reports the same way on every OS with a port (`mem.rs` on
//! Windows, `mem_mac.rs` on macOS): the units, the breakdown's row shapes, and
//! everything read from an allocator or a live structure rather than from the
//! OS — JSC's heap and its mimalloc, gpui's atlas and decoded images, the
//! retained tree, vn-tree's side tables, the devtools' rings, the Rust heap.
//!
//! The OS half (the process counters, the address-space walk, the system
//! allocator, the GPU) and the fold into categories stay with each port.

use gpui::{App, Window};
use vn_runtime::JsRuntime;

use crate::json::Json;

// mimalloc is JSC's allocator here (redjs compiles it into its static lib, on
// every OS).
extern "C" {
    fn mi_stats_get_json(buf_size: usize, buf: *mut std::ffi::c_char) -> *mut std::ffi::c_char;
}

pub(super) fn mb(bytes: f64) -> Json {
    Json::n((bytes / 1048576.0 * 10.0).round() / 10.0)
}

pub(super) fn bytes(v: f64) -> Json {
    Json::n(v.max(0.0).round())
}

pub(super) fn ms(t: std::time::Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

pub(super) fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
}

// ---------------------------------------------------------------------------
// the breakdown's rows
// ---------------------------------------------------------------------------

pub(super) fn item(label: impl Into<String>, b: f64) -> Json {
    Json::obj(vec![("label", Json::s(label)), ("bytes", bytes(b))])
}

/// A row that is NOT a slice of its category — a figure from another
/// accountant (JSC's own GC heap count) shown beside the slices it overlaps.
pub(super) fn info(label: impl Into<String>, b: f64) -> Json {
    Json::obj(vec![
        ("label", Json::s(label)),
        ("bytes", bytes(b)),
        ("info", Json::Bool(true)),
    ])
}

pub(super) fn category(key: &str, label: &str, b: f64, note: &str, items: Vec<Json>) -> Json {
    Json::obj(vec![
        ("key", Json::s(key)),
        ("label", Json::s(label)),
        ("bytes", bytes(b)),
        ("note", Json::s(note)),
        ("items", Json::arr(items)),
    ])
}

/// Adds a category's resident bytes (all, and private) when they were counted.
pub(super) fn with_resident(mut c: Json, r: Option<[f64; 2]>) -> Json {
    if let (Json::Obj(fields), Some([private, shared])) = (&mut c, r) {
        fields.push(("residentBytes".into(), bytes(private + shared)));
        fields.push(("residentPrivateBytes".into(), bytes(private)));
    }
    c
}

// ---------------------------------------------------------------------------
// JSC and its malloc
// ---------------------------------------------------------------------------

/// One `"name": { "total": .., "peak": .., "current": N }` out of mimalloc's
/// JSON, by name.
pub(super) fn mi_current(json: &str, name: &str) -> Option<f64> {
    let at = json.find(&format!("\"{name}\":"))?;
    let rest = &json[at..];
    let cur = rest.find("\"current\":")? + "\"current\":".len();
    let digits: String = rest[cur..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().ok()
}

pub(super) struct Mimalloc {
    pub(super) committed: f64,
    /// Live allocations: `malloc_normal` + `malloc_huge`.
    pub(super) live: f64,
    pub(super) json: Json,
}

pub(super) fn mimalloc() -> Mimalloc {
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

pub(super) struct Jsc {
    pub(super) size: f64,
    pub(super) capacity: f64,
    pub(super) extra: f64,
    pub(super) json: Json,
}

pub(super) fn jsc_stats() -> Jsc {
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

/// `release_memory` (two full synchronous collections, then free pages back
/// to the OS) — or, with `code`, the same after deleting all compiled code
/// that is not running. Returns how long it took, in ms.
pub(super) fn collect(code: bool) -> f64 {
    let t = std::time::Instant::now();
    JsRuntime::with(|rt| unsafe {
        if code {
            vn_runtime::redjs::ffi::redjs_release_memory_and_code(rt.raw())
        } else {
            vn_runtime::redjs::ffi::redjs_release_memory(rt.raw())
        }
    });
    ms(t)
}

// ---------------------------------------------------------------------------
// the renderer
// ---------------------------------------------------------------------------

pub(super) struct Renderer {
    pub(super) targets: f64,
    /// Atlas texture bytes per kind: monochrome, polychrome, subpixel.
    pub(super) atlas: [f64; 3],
    pub(super) json: Json,
}

/// The primary window's renderer, from gpui (fork accessors) and its size.
pub(super) fn renderer(window: &Window) -> Renderer {
    let scale = window.scale_factor();
    let size = window.viewport_size();
    let (w, h) = (
        (f32::from(size.width) * scale).round() as f64,
        (f32::from(size.height) * scale).round() as f64,
    );
    // 32 bytes a pixel on both backends: `gpui_windows::directx_renderer` has
    // a 3-buffer BGRA swap chain, a single-sample and a 4x MSAA BGRA path
    // target; `gpui_apple::metal_renderer` a CAMetalLayer of 3 BGRA drawables,
    // a path intermediate texture and its 4x MSAA twin.
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

/// The GPU block's named rows: the atlas per kind and the render targets.
pub(super) fn gpu_items(r: &Renderer) -> Vec<Json> {
    let a = r.atlas;
    vec![
        item("sprite atlas: monochrome (glyphs)", a[0]),
        item("sprite atlas: polychrome (images, emoji, SVG)", a[1]),
        item("sprite atlas: subpixel (glyphs)", a[2]),
        item(
            "render targets (swap chain + path MSAA, estimate)",
            r.targets,
        ),
    ]
}

// ---------------------------------------------------------------------------
// what the host's own structures hold
// ---------------------------------------------------------------------------

pub(super) struct Images {
    pub(super) decoded: f64,
    pub(super) copies: f64,
    pub(super) json: Json,
}

pub(super) fn images(cx: &App) -> Images {
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

pub(super) fn tree() -> (Json, f64) {
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
pub(super) fn side() -> Vec<(&'static str, u64)> {
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

pub(super) fn side_json(side: &[(&'static str, u64)]) -> Json {
    Json::obj(side.iter().map(|(k, v)| (*k, Json::n(*v as f64))).collect())
}

/// `<vn-sticker>`'s engine: its sources, loaded engines, frames held and loop
/// cache. `null` until something has used it.
pub(super) fn stickers() -> Json {
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

pub(super) fn stylesheet_compiles() -> Json {
    let (compiles, compile_ms, compile_source, compile_growth) = vn_tree::perf::sheet_compiles();
    Json::obj(vec![
        ("compiles", Json::n(compiles as f64)),
        ("ms", Json::n(compile_ms.round())),
        ("sourceBytes", bytes(compile_source as f64)),
        (
            "netGrowthBytes",
            compile_growth.map_or(Json::Null, |g| Json::n(g as f64)),
        ),
    ])
}

pub(super) fn rust_json(rust: Option<(i64, i64, i64, u64)>) -> Json {
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

/// The Rust heap's slices, each computed from the live structure, then what
/// the stylesheet compiles left behind, then the rest of `live` nothing
/// itemises. The OS's system allocator rows (C/C++ malloc on the same heap,
/// slack) follow them in each port's `breakdown`.
pub(super) fn rust_items(live: f64, tree_bytes: f64, images: &Images, side: &[(&'static str, u64)]) -> Vec<Json> {
    let side = |name: &str| {
        side
            .iter()
            .find(|(k, _)| *k == name)
            .map_or(0.0, |(_, v)| *v as f64)
    };
    let mut rust_items = Vec::new();
    let parts = [
        (
            "retained tree (nodes, styles, vars) — estimate",
            tree_bytes,
        ),
        (
            "decoded images (gpui asset cache; a reduced one is its copy's size)",
            images.decoded,
        ),
        ("downscaled image copies (resample LRU)", images.copies),
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
    rust_items
}
